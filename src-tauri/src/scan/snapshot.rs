//! A compact picture of known disk usage for the AI advisor.

use std::cmp::Reverse;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use super::directory::fill_stored_sizes;
use super::listing::is_real_dir;
use super::lock;
use super::tree::{Node, Tree};
use super::types::{SnapshotItem, StorageSnapshot};
use super::walk::measure_tree;
use super::{STORED, STORED_LARGEST, stored_for};
use crate::platform::{self, allocated_bytes, list_children};

/// Gather a structured snapshot of known disk usage, current directory items, stored tree folders,
/// largest files, and common user cache/cleanup locations.
pub fn storage_snapshot(current_path: Option<&str>) -> StorageSnapshot {
    let disk = platform::volume_summary(None).ok();
    let current_path_str = current_path.map(ToOwned::to_owned);

    let mut current_items = Vec::new();
    if let Some(dir_str) = current_path {
        let dir = Path::new(dir_str);
        if let Ok(mut children) = list_children(dir) {
            let _ = fill_stored_sizes(stored_for(dir), dir, &mut children, false);
            for child in children {
                if let Some(bytes) = child.bytes {
                    current_items.push(SnapshotItem {
                        path: child.path,
                        name: child.name,
                        is_dir: child.is_dir,
                        bytes,
                        own_bytes: if child.is_dir { 0 } else { bytes },
                    });
                }
            }
            current_items.sort_by_key(|item| Reverse(item.bytes));
            current_items.truncate(35);
        }
    }

    let mut top_folders = Vec::new();
    let mut dense_folders = Vec::new();
    for slot in &STORED {
        if let Some(tree) = lock(slot).as_ref() {
            top_folders.extend(ranked_folders(tree, 40, 0, |node| node.total));
            dense_folders.extend(ranked_folders(tree, 25, 10 * 1024 * 1024, |node| node.own));
        }
    }
    top_folders.sort_by_key(|item| Reverse(item.bytes));
    top_folders.truncate(40);
    dense_folders.sort_by_key(|item| Reverse(item.own_bytes));
    dense_folders.truncate(25);

    let mut largest_files = lock(&STORED_LARGEST).clone();
    largest_files.truncate(40);

    let mut notable_locations = Vec::new();
    for notable in platform::notable_locations() {
        if !notable.path.exists() {
            continue;
        }
        let known_bytes = lock(stored_for(&notable.path)).as_ref().and_then(|tree| {
            tree.find(&notable.path)
                .and_then(|id| tree.size_of(id).ok())
        });
        let bytes = known_bytes.or_else(|| {
            notable
                .measure
                .then(|| measure_tree(&notable.path, None, &Arc::default()).ok())
                .flatten()
                .and_then(|t| t.size_of(0).ok())
        });
        if let Some(bytes) = bytes
            && bytes > 0
        {
            notable_locations.push(SnapshotItem {
                path: notable.path.to_string_lossy().into_owned(),
                name: notable.label,
                is_dir: true,
                bytes,
                own_bytes: 0,
            });
        }
    }
    notable_locations.sort_by_key(|item| Reverse(item.bytes));

    StorageSnapshot {
        disk,
        current_path: current_path_str,
        current_items,
        top_folders,
        dense_folders,
        largest_files,
        notable_locations,
    }
}

/// The `limit` folders with the most `key` bytes above `min`. A whole-disk tree holds millions of
/// folders, so paths are built only for the ones kept.
pub(super) fn ranked_folders(
    tree: &Tree,
    limit: usize,
    min: u64,
    key: impl Fn(&Node) -> u64,
) -> Vec<SnapshotItem> {
    let skip_root = platform::is_volume_root(&tree.root);
    let mut ids: Vec<u32> = (0..tree.nodes.len() as u32)
        .filter(|&id| !(id == 0 && skip_root) && key(&tree.nodes[id as usize]) > min)
        .collect();
    let by_size = |id: &u32| Reverse(key(&tree.nodes[*id as usize]));
    if ids.len() > limit {
        ids.select_nth_unstable_by_key(limit, by_size);
        ids.truncate(limit);
    }
    ids.sort_unstable_by_key(by_size);
    ids.into_iter()
        .map(|id| {
            let node = &tree.nodes[id as usize];
            let path = tree.path_of(id);
            let name = if id == 0 {
                path.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned())
            } else {
                node.name.to_string()
            };
            SnapshotItem {
                path: path.to_string_lossy().into_owned(),
                name,
                is_dir: true,
                bytes: node.total,
                own_bytes: node.own,
            }
        })
        .collect()
}

/// Inspect a path on disk, returning `(allocated_bytes, is_dir)` if it exists.
pub fn inspect_path_size(path: &Path) -> Option<(u64, bool)> {
    let metadata = fs::symlink_metadata(path).ok()?;
    let is_dir = is_real_dir(&metadata.file_type());
    if !is_dir {
        let bytes = if metadata.is_file() {
            allocated_bytes(path, &metadata)
        } else {
            0
        };
        return Some((bytes, false));
    }
    {
        let guard = lock(stored_for(path));
        if let Some(tree) = guard.as_ref()
            && let Some(id) = tree.find(path)
            && let Ok(bytes) = tree.size_of(id)
        {
            return Some((bytes, true));
        }
    }
    let tree = measure_tree(path, None, &Arc::default()).ok()?;
    let bytes = tree.size_of(0).unwrap_or(0);
    Some((bytes, true))
}
