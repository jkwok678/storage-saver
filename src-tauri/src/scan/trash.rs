//! Move an item to the Trash and drop it from the stored totals.

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::listing::is_real_dir;
use super::lock;
use super::tree::Tree;
use super::types::{LargestFile, TrashOutcome};
use super::walk::measure_tree;
use super::{STORED_LARGEST, stored_for};
use crate::platform::{self, allocated_bytes, lowercase};

pub fn is_protected_path(path: &Path) -> bool {
    platform::is_protected_path(path)
}

pub(super) async fn move_to_trash(path: String) -> Result<TrashOutcome, String> {
    let path = platform::normalize_input(&path)?;
    match tauri::async_runtime::spawn_blocking(move || {
        let target = Path::new(&path);
        let trash_dir = platform::trash_dir_for(target)?;
        trash_path_into(
            target,
            &trash_dir,
            stored_for(target),
            &STORED_LARGEST,
            platform::move_to_trash,
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
    let resolved = match (target.parent().map(dunce::canonicalize), target.file_name()) {
        (Some(Ok(parent)), Some(name)) => parent.join(name),
        _ => target.to_path_buf(),
    };
    if is_protected_path(target) || is_protected_path(&resolved) {
        return Err(format!(
            "Refusing to move protected folder {}",
            target.display()
        ));
    }
    let trash_resolved = dunce::canonicalize(trash_dir).unwrap_or_else(|_| trash_dir.to_path_buf());
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
        allocated_bytes(target, &metadata)
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
        largest.retain(|file| !Path::new(&file.path).starts_with(target));
    }

    Ok(TrashOutcome {
        original_path: target.to_string_lossy().into_owned(),
        trash_path: dest.to_string_lossy().into_owned(),
        freed_bytes,
        put_back,
    })
}

/// Rename into `trash_dir`, numbering the name when the Trash already holds one like it.
/// Windows never falls back to a plain move: the Recycle Bin is not a folder to rename into.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn move_into_trash(target: &Path, trash_dir: &Path) -> Result<PathBuf, String> {
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

#[cfg(any(target_os = "macos", test))]
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
