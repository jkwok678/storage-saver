//! Fixed drives and their capacity.

use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    GetVolumeNameForVolumeMountPointW,
};
use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;

use crate::platform::{Volume, winpath};
use crate::scan::DiskSummary;

/// `X:\` as null-terminated UTF-16.
fn root_wide(drive: u8) -> Vec<u16> {
    format!("{}:\\", char::from(b'A' + drive))
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// Total and available bytes for the caller, from the volume at `root`.
fn disk_space(root: &[u16]) -> io::Result<(u64, u64)> {
    let mut available = 0u64;
    let mut total = 0u64;
    // SAFETY: root is null-terminated, and the out-pointers are valid. The total of free
    // bytes, which includes quota the caller cannot use, is not wanted.
    let ok =
        unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut available, &mut total, ptr::null_mut()) };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok((total, available))
    }
}

/// The volume's unique name. Fails for a SUBST drive, which has no volume of its own.
fn volume_guid(root: &[u16]) -> Option<String> {
    let mut buffer = [0u16; 64];
    // SAFETY: root is null-terminated, and the buffer length passed is its real length.
    let ok = unsafe {
        GetVolumeNameForVolumeMountPointW(root.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32)
    };
    (ok != 0).then(|| from_wide(&buffer))
}

fn volume_label(root: &[u16]) -> Option<String> {
    let mut buffer = [0u16; 261];
    // SAFETY: root is null-terminated and the buffer length passed is its real length. The
    // other outputs are not wanted.
    let ok = unsafe {
        GetVolumeInformationW(
            root.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            0,
        )
    };
    (ok != 0)
        .then(|| from_wide(&buffer))
        .filter(|label| !label.is_empty())
}

/// Every fixed drive that has a letter, each volume once.
pub(crate) fn volumes() -> Vec<Volume> {
    // SAFETY: takes no arguments.
    let letters = unsafe { GetLogicalDrives() };
    let mut seen = HashSet::new();
    let mut volumes = Vec::new();
    for drive in 0..26u8 {
        if letters & (1 << drive) == 0 {
            continue;
        }
        let root = root_wide(drive);
        // SAFETY: root is null-terminated.
        if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_FIXED {
            continue;
        }
        let Some(guid) = volume_guid(&root) else {
            continue;
        };
        if !seen.insert(guid) {
            continue;
        }
        let letter = char::from(b'A' + drive);
        let label = volume_label(&root).unwrap_or_else(|| "Local Disk".to_string());
        let (total, available, error) = match disk_space(&root) {
            Ok((total, available)) => (total, available, None),
            Err(err) => (0, 0, Some(err.to_string())),
        };
        volumes.push(Volume {
            path: PathBuf::from(format!("{letter}:\\")),
            label: format!("{label} ({letter}:)"),
            total,
            available,
            error,
        });
    }
    volumes
}

fn summary(total: u64, available: u64) -> DiskSummary {
    DiskSummary {
        total,
        available,
        used: total.saturating_sub(available),
    }
}

/// The volume holding `path`, or every fixed drive together for no path or the top level.
pub(crate) fn volume_summary(path: Option<&Path>) -> Result<DiskSummary, String> {
    if let Some(path) = path.filter(|path| !path.as_os_str().is_empty()) {
        let drive = path
            .to_str()
            .and_then(winpath::drive_index)
            .ok_or_else(|| "Unsupported path".to_string())?;
        let (total, available) = disk_space(&root_wide(drive)).map_err(|err| err.to_string())?;
        return Ok(summary(total, available));
    }
    let (mut total, mut available) = (0u64, 0u64);
    for volume in volumes().iter().filter(|volume| volume.error.is_none()) {
        total = total.saturating_add(volume.total);
        available = available.saturating_add(volume.available);
    }
    Ok(summary(total, available))
}
