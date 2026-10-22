//! One folder's children, and the boot volume's free space.

use std::ffi::CString;
use std::fs::{self, FileType};
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::tree::{DENIED, MeasureError, Trouble};
use super::types::{ChildEntry, DiskSummary};

pub(super) fn read_disk_summary() -> Result<DiskSummary, String> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    let path = CString::new("/").map_err(|err| err.to_string())?;
    // SAFETY: stat is initialized by statfs before it is read, and the path is a valid C string.
    let rc = unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: statfs returned success, so every field is initialized.
    let stat = unsafe { stat.assume_init() };
    let block = u64::from(stat.f_bsize);
    let total = stat.f_blocks.saturating_mul(block);
    let available = stat.f_bavail.saturating_mul(block);
    let used = total.saturating_sub(available);
    Ok(DiskSummary {
        total,
        available,
        used,
    })
}

pub(super) fn list_children(root: &Path) -> Result<Vec<ChildEntry>, String> {
    let entries = fs::read_dir(root).map_err(|err| {
        if err.kind() == ErrorKind::PermissionDenied {
            "Not permitted".to_string()
        } else {
            err.to_string()
        }
    })?;

    let mut children = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if is_excluded(&path) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata.file_type(),
                Err(_) => continue,
            },
        };
        let is_dir = is_real_dir(&file_type);
        let metadata = fs::symlink_metadata(&path);
        let modified = metadata.as_ref().ok().and_then(modified_secs);
        let (bytes, error) = if is_dir {
            (None, None)
        } else if let Ok(metadata) = &metadata {
            // Links, sockets and the like take no space of their own worth showing.
            let bytes = if file_type.is_file() {
                allocated_bytes(metadata)
            } else {
                0
            };
            (Some(bytes), None)
        } else if !file_type.is_file() {
            (Some(0), None)
        } else {
            let mut trouble = Trouble::default();
            if let Err(err) = &metadata {
                trouble.note_io(err);
            }
            if trouble.flags & DENIED != 0 {
                (None, Some(MeasureError::NotPermitted.text()))
            } else {
                (Some(0), trouble.message)
            }
        };
        children.push(ChildEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            is_dir,
            bytes,
            modified,
            error,
        });
    }
    Ok(children)
}

/// Other mounts that would count the same files twice, or that are not this boot disk.
/// `/System/Volumes` repeats `/Users`, `/Applications`, and the rest through APFS firmlinks.
pub(super) fn is_excluded(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor == Path::new("/System/Volumes")
            || ancestor == Path::new("/Volumes")
            || ancestor == Path::new("/dev")
            || ancestor == Path::new("/.vol")
    })
}

pub(super) fn is_real_dir(file_type: &FileType) -> bool {
    file_type.is_dir() && !file_type.is_symlink()
}

/// Allocated space, in bytes. `st_blocks` is reported in 512-byte units.
pub(super) fn allocated_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

fn modified_secs(metadata: &fs::Metadata) -> Option<u64> {
    u64::try_from(metadata.mtime()).ok()
}
