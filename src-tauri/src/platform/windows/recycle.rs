//! Protected folders and moving items to the Recycle Bin through the shell.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows_sys::Win32::UI::Shell::{
    FO_DELETE, FOF_ALLOWUNDO, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING, SHFILEOPSTRUCTW,
    SHFileOperationW,
};

use crate::platform::winpath::{self, ProtectedEnv};

/// The shell's delete takes paths of at most this many characters, with the terminator.
const SHELL_PATH_LIMIT: usize = 260;

fn env_lower(name: &str) -> String {
    env::var(name)
        .ok()
        .and_then(|value| winpath::normalize(&value))
        .map(|value| value.to_lowercase())
        .unwrap_or_default()
}

pub(crate) fn is_protected_path(path: &Path) -> bool {
    // Anything that is not a plain drive path, such as a network path, is not ours to move.
    let Some(plain) = path.to_str().and_then(winpath::normalize) else {
        return true;
    };
    let env = ProtectedEnv {
        system_root: env_lower("SystemRoot"),
        program_files: env_lower("ProgramFiles"),
        program_files_x86: env_lower("ProgramFiles(x86)"),
        program_data: env_lower("ProgramData"),
        profile: env_lower("USERPROFILE"),
    };
    winpath::is_protected(&plain.to_lowercase(), &env)
}

/// The Recycle Bin folder on the drive of `target`.
pub(crate) fn trash_dir_for(target: &Path) -> Result<PathBuf, String> {
    let drive = target
        .to_str()
        .and_then(winpath::drive_index)
        .ok_or_else(|| "Could not determine the drive for this path".to_string())?;
    Ok(PathBuf::from(format!(
        "{}:\\$Recycle.Bin",
        char::from(b'A' + drive)
    )))
}

/// Recycle through the shell, which records where the item came from so "Restore" works.
/// Never deletes for good: an item the Recycle Bin cannot take stays where it is.
pub(crate) fn move_to_trash(target: &Path, trash_dir: &Path) -> Result<(PathBuf, bool), String> {
    let plain = target
        .to_str()
        .and_then(winpath::normalize)
        .ok_or_else(|| "Unsupported path".to_string())?;
    if plain.encode_utf16().count() >= SHELL_PATH_LIMIT {
        return Err(
            "This path is too long for the Recycle Bin. Move it in File Explorer.".to_string(),
        );
    }
    // pFrom is a list of paths, each ending in a null, and the list ends in a second one.
    let from: Vec<u16> = plain.encode_utf16().chain([0, 0]).collect();

    // The shell wants COM, and COM state belongs to a thread, so it gets one of its own.
    let (code, aborted) = std::thread::spawn(move || shell_delete(&from))
        .join()
        .map_err(|_| "The Recycle Bin operation failed unexpectedly".to_string())?;

    if code != 0 {
        return Err(format!(
            "Windows could not move this item to the Recycle Bin (error {code:#x})."
        ));
    }
    if aborted {
        return Err("Moving to the Recycle Bin was cancelled.".to_string());
    }
    if fs::symlink_metadata(target).is_ok() {
        return Err(
            "The item is still in place. The Recycle Bin may be turned off for this drive, or the item is too large for it."
                .to_string(),
        );
    }
    Ok((trash_dir.to_path_buf(), true))
}

/// Runs the shell delete with undo, so the item goes to the Recycle Bin. Returns the result
/// code and whether the user aborted it.
fn shell_delete(from: &[u16]) -> (i32, bool) {
    // Without FOF_NOCONFIRMATION, an item the Recycle Bin cannot take raises the shell's
    // "delete permanently?" warning instead of being deleted silently.
    let mut op = SHFILEOPSTRUCTW {
        hwnd: ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: ptr::null(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOERRORUI | FOF_SILENT | FOF_WANTNUKEWARNING) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: ptr::null_mut(),
        lpszProgressTitle: ptr::null(),
    };
    // SAFETY: COM is initialized for this thread and released before it ends. `op` is fully
    // initialized, and `from` holds the double-null-terminated list for the whole call.
    unsafe {
        let com = CoInitializeEx(ptr::null(), COINIT_APARTMENTTHREADED as u32) >= 0;
        let code = SHFileOperationW(&mut op);
        if com {
            CoUninitialize();
        }
        (code, op.fAnyOperationsAborted != 0)
    }
}
