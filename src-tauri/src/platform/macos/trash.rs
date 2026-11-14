//! Protected folders and moving items to the Trash through Finder.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::home_dir;
use crate::platform::lowercase;
use crate::scan::trash::move_into_trash;

pub(crate) fn is_protected_path(path: &Path) -> bool {
    // APFS ignores case by default, so `/system` and `~/library` name the protected folders too.
    let path = &lowercase(path);
    let protected_exact = [
        "/",
        "/System",
        "/Library",
        "/Applications",
        "/Users",
        "/bin",
        "/sbin",
        "/usr",
        "/var",
        "/private",
        "/private/var",
        "/private/etc",
        "/etc",
        "/dev",
        "/Volumes",
        "/cores",
        "/opt",
        "/Users/Shared",
    ];
    for entry in protected_exact {
        if *path == lowercase(Path::new(entry)) {
            return true;
        }
    }
    if path.starts_with("/system") || path.starts_with("/bin") || path.starts_with("/sbin") {
        return true;
    }
    if let Some(home) = home_dir() {
        let home_path = lowercase(&home);
        if *path == home_path {
            return true;
        }
        for top in [
            "Applications",
            "Desktop",
            "Documents",
            "Downloads",
            "Library",
            "Library/Application Support",
            "Library/Containers",
            "Library/Group Containers",
            "Library/Keychains",
            "Library/Mail",
            "Library/Messages",
            "Library/Mobile Documents",
            "Library/Preferences",
            "Pictures",
            "Music",
            "Movies",
            ".Trash",
        ] {
            if *path == home_path.join(top.to_lowercase()) {
                return true;
            }
        }
    }
    false
}

/// The Trash folder for `target`.
pub(crate) fn trash_dir_for(_target: &Path) -> Result<PathBuf, String> {
    home_dir()
        .map(|home| home.join(".Trash"))
        .ok_or_else(|| "Could not determine HOME directory".to_string())
}

/// Trash through Finder, which records where the item came from so "Put Back" works.
/// Falls back to a plain move when Finder cannot do it, e.g. when Automation is not allowed.
pub(crate) fn move_to_trash(target: &Path, trash_dir: &Path) -> Result<(PathBuf, bool), String> {
    // Finder resolves a link to what it points to, so a link is moved itself instead.
    let is_link = fs::symlink_metadata(target).is_ok_and(|m| m.file_type().is_symlink());
    if !is_link {
        match finder_trash(target) {
            Ok(dest) => return Ok((dest, true)),
            // Finder trashed it and only its reply went wrong.
            Err(_) if fs::symlink_metadata(target).is_err() => {
                let name = target.file_name().unwrap_or_default();
                return Ok((trash_dir.join(name), true));
            }
            Err(_) => {}
        }
    }
    move_into_trash(target, trash_dir).map(|dest| (dest, false))
}

fn finder_trash(target: &Path) -> Result<PathBuf, String> {
    // The path goes in as an argument, never into the script text.
    let output = Command::new("/usr/bin/osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "set target to POSIX file (item 1 of argv) as alias",
            "-e",
            "tell application \"Finder\" to set trashed to (delete target) as alias",
            "-e",
            "return POSIX path of trashed",
            "-e",
            "end run",
        ])
        .arg(target)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let dest = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let dest = dest.strip_suffix('/').unwrap_or(&dest);
    Ok(PathBuf::from(dest))
}
