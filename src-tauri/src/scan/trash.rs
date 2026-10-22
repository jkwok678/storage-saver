//! Move an item to the Trash and drop it from the stored totals.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use super::listing::{allocated_bytes, is_real_dir};
use super::lock;
use super::tree::Tree;
use super::types::{LargestFile, TrashOutcome};
use super::walk::measure_tree;
use super::{STORED, STORED_LARGEST};

pub fn is_protected_path(path: &Path) -> bool {
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
    if let Ok(home) = std::env::var("HOME") {
        let home_path = lowercase(Path::new(&home));
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

fn lowercase(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

pub(super) async fn move_to_trash(path: String) -> Result<TrashOutcome, String> {
    match tauri::async_runtime::spawn_blocking(move || {
        let home =
            std::env::var("HOME").map_err(|_| "Could not determine HOME directory".to_string())?;
        let trash_dir = PathBuf::from(home).join(".Trash");
        trash_path_into(
            Path::new(&path),
            &trash_dir,
            &STORED,
            &STORED_LARGEST,
            move_with_finder,
        )
    })
    .await
    {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

/// Move `target` to the Trash with `move_item`, which returns where it landed and whether
/// "Put Back" can restore it, then drop it from the stored totals.
pub fn trash_path_into(
    target: &Path,
    trash_dir: &Path,
    stored: &Mutex<Option<Tree>>,
    stored_largest: &Mutex<Vec<LargestFile>>,
    move_item: impl FnOnce(&Path, &Path) -> Result<(PathBuf, bool), String>,
) -> Result<TrashOutcome, String> {
    if !target.is_absolute() {
        return Err("Path must be absolute".into());
    }
    // `..` would let a path that reads as harmless land on a protected folder.
    if target.components().any(|c| c == Component::ParentDir) {
        return Err("Path must not contain `..`".into());
    }
    let metadata = fs::symlink_metadata(target).map_err(|err| err.to_string())?;
    // Check the folder the item really sits in, so a linked parent cannot hide a protected one.
    // The item itself is not resolved: a link goes to the Trash, not what it points to.
    let resolved = match (target.parent().map(fs::canonicalize), target.file_name()) {
        (Some(Ok(parent)), Some(name)) => parent.join(name),
        _ => target.to_path_buf(),
    };
    if is_protected_path(target) || is_protected_path(&resolved) {
        return Err(format!(
            "Refusing to move protected folder {}",
            target.display()
        ));
    }
    let trash_resolved = fs::canonicalize(trash_dir).unwrap_or_else(|_| trash_dir.to_path_buf());
    if lowercase(target).starts_with(lowercase(trash_dir))
        || lowercase(&resolved).starts_with(lowercase(&trash_resolved))
    {
        return Err("Item is already in the Trash".into());
    }
    let is_dir = is_real_dir(&metadata.file_type());
    let freed_bytes = if is_dir {
        let from_stored = lock(stored)
            .as_ref()
            .and_then(|tree| tree.find(target).and_then(|id| tree.size_of(id).ok()));
        from_stored.unwrap_or_else(|| {
            measure_tree(target, None, &Arc::default())
                .ok()
                .and_then(|t| t.size_of(0).ok())
                .unwrap_or(0)
        })
    } else if metadata.is_file() {
        allocated_bytes(&metadata)
    } else {
        0
    };

    let (dest, put_back) = move_item(target, trash_dir)?;

    // Keep stored tree and largest files in sync with the deletion.
    {
        let mut guard = lock(stored);
        if guard
            .as_ref()
            .is_some_and(|tree| tree.root.starts_with(target))
        {
            // The stored root went with it, so nothing the tree holds is on disk any more.
            *guard = None;
        } else if let Some(tree) = guard.as_mut() {
            if is_dir {
                if let Some(id) = tree.find(target) {
                    tree.remove_node(id);
                }
            } else if let Some(parent) = target.parent()
                && let Some(parent_id) = tree.find(parent)
            {
                let node = &mut tree.nodes[parent_id as usize];
                node.own = node.own.saturating_sub(freed_bytes);
                tree.roll_up();
            }
        }
    }
    {
        let mut largest = lock(stored_largest);
        let prefix = format!("{}/", target.to_string_lossy());
        let exact = target.to_string_lossy();
        largest.retain(|file| file.path != exact.as_ref() && !file.path.starts_with(&prefix));
    }

    Ok(TrashOutcome {
        original_path: target.to_string_lossy().into_owned(),
        trash_path: dest.to_string_lossy().into_owned(),
        freed_bytes,
        put_back,
    })
}

/// Trash through Finder, which records where the item came from so "Put Back" works.
/// Falls back to a plain move when Finder cannot do it, e.g. when Automation is not allowed.
fn move_with_finder(target: &Path, trash_dir: &Path) -> Result<(PathBuf, bool), String> {
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

/// Rename into `trash_dir`, numbering the name when the Trash already holds one like it.
pub(super) fn move_into_trash(target: &Path, trash_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(trash_dir).map_err(|err| err.to_string())?;
    let file_name = target
        .file_name()
        .ok_or_else(|| "Invalid file name".to_string())?
        .to_string_lossy()
        .into_owned();
    let dest = unique_trash_destination(trash_dir, &file_name);
    fs::rename(target, &dest).map_err(|err| err.to_string())?;
    Ok(dest)
}

fn unique_trash_destination(trash_dir: &Path, name: &str) -> PathBuf {
    let initial = trash_dir.join(name);
    if fs::symlink_metadata(&initial).is_err() {
        return initial;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    for counter in 2..10_000 {
        let candidate = trash_dir.join(format!("{stem} {counter}{ext}"));
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    trash_dir.join(format!("{stem}-{}{ext}", std::process::id()))
}
