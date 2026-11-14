//! Browse one folder: list it, reuse stored totals, and walk only what is missing.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use tauri::ipc::Channel;

use super::lock;
use super::tree::{MeasureError, Tree};
use super::types::{ChildEntry, ScanEvent};
use super::walk::{measure_tree, worker_count};
use super::{SCAN_GENERATION, cancel_if_current, scan_cancelled, stored_for};
use crate::platform::list_children;

pub(super) async fn scan_directory(
    path: String,
    refresh: bool,
    on_event: Channel<ScanEvent>,
) -> Result<(), String> {
    // The walk is long. Run it off the UI thread so folder names and sizes can paint as they arrive.
    match tauri::async_runtime::spawn_blocking(move || run_scan(path, refresh, on_event)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

fn run_scan(path: String, refresh: bool, on_event: Channel<ScanEvent>) -> Result<(), String> {
    let generation = SCAN_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let root = PathBuf::from(&path);
    if !root.is_absolute() {
        return Err("path must be absolute".into());
    }
    scan_into(
        stored_for(&root),
        &root,
        refresh,
        Some(generation),
        |event| on_event.send(event).is_ok(),
    )
}

/// List `root`, fill in what the stored tree already knows, and walk only the folders it
/// does not have (or every folder, on `refresh`). `send` returns false once nobody is listening.
pub(super) fn scan_into(
    stored: &Mutex<Option<Tree>>,
    root: &Path,
    refresh: bool,
    generation: Option<u64>,
    mut send: impl FnMut(ScanEvent) -> bool,
) -> Result<(), String> {
    let cancelled = || generation.is_some_and(scan_cancelled);
    let stop = || {
        if let Some(generation) = generation {
            cancel_if_current(generation);
        }
    };

    let mut children = list_children(root)?;
    children.sort_by_cached_key(|entry| entry.name.to_lowercase());
    let pending = fill_stored_sizes(stored, root, &mut children, refresh);

    if cancelled() {
        return Ok(());
    }
    if !send(ScanEvent::Started {
        entries: children.clone(),
    }) {
        stop();
        return Ok(());
    }

    let walk_needed = refresh || !pending.is_empty();
    let walked = if pending.is_empty() {
        HashMap::new()
    } else {
        match walk_folders(pending, generation, &mut send) {
            Some(walked) => walked,
            None => return Ok(()),
        }
    };

    if cancelled() {
        return Ok(());
    }
    if walk_needed {
        store_scan(stored, generation, root, &children, walked);
    }
    if !send(ScanEvent::Done) {
        stop();
    }
    Ok(())
}

/// Give each stored folder its size. Returns the folders that still need a walk.
pub(super) fn fill_stored_sizes(
    stored: &Mutex<Option<Tree>>,
    root: &Path,
    children: &mut [ChildEntry],
    refresh: bool,
) -> Vec<String> {
    let stored = lock(stored);
    let tree = stored.as_ref().filter(|_| !refresh);
    let ids = tree
        .map(|tree| tree.find_children(root, children))
        .unwrap_or_default();

    let mut pending = Vec::new();
    for (index, child) in children.iter_mut().enumerate() {
        if !child.is_dir {
            continue;
        }
        match (tree, ids.get(index).copied().flatten()) {
            (Some(tree), Some(id)) => {
                let (bytes, error) = size_fields(tree.size_of(id));
                child.bytes = Some(bytes);
                child.error = error;
            }
            _ => pending.push(child.path.clone()),
        }
    }
    pending
}

/// Walk each folder on its own worker and report each total as it lands.
/// Returns `None` when the scan stopped early.
fn walk_folders(
    folders: Vec<String>,
    generation: Option<u64>,
    send: &mut impl FnMut(ScanEvent) -> bool,
) -> Option<HashMap<String, Tree>> {
    // Shared across the folders so a file linked from two of them counts once.
    let links = Arc::new(Mutex::new(HashSet::new()));
    let workers = worker_count(folders.len());
    let queue = Mutex::new(folders);
    let mut walked = HashMap::new();
    let mut completed = true;

    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..workers {
            let tx = tx.clone();
            let queue = &queue;
            let links = &links;
            scope.spawn(move || {
                loop {
                    if generation.is_some_and(scan_cancelled) {
                        break;
                    }
                    let Some(path) = lock(queue).pop() else { break };
                    let measured = measure_tree(Path::new(&path), generation, links);
                    if tx.send((path, measured)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        while let Ok((path, measured)) = rx.recv() {
            if generation.is_some_and(scan_cancelled) {
                completed = false;
                break;
            }
            let Ok(tree) = measured else {
                completed = false;
                break;
            };
            let (bytes, error) = size_fields(tree.size_of(0));
            walked.insert(path.clone(), tree);
            if !send(ScanEvent::Sized { path, bytes, error }) {
                if let Some(generation) = generation {
                    cancel_if_current(generation);
                }
                completed = false;
                break;
            }
        }
    });

    completed.then_some(walked)
}

/// Build the tree for `root` from fresh walks and stored folders, and keep it.
fn store_scan(
    stored: &Mutex<Option<Tree>>,
    generation: Option<u64>,
    root: &Path,
    children: &[ChildEntry],
    mut walked: HashMap<String, Tree>,
) {
    let mut stored = lock(stored);
    // Checked under the lock, so a scan that was replaced cannot overwrite the newer one's tree.
    if generation.is_some_and(scan_cancelled) {
        return;
    }

    let mut tree = Tree::new(root.to_path_buf());
    let stored_ids = stored
        .as_ref()
        .map(|old| old.find_children(root, children))
        .unwrap_or_default();
    for (index, child) in children.iter().enumerate() {
        if !child.is_dir {
            tree.nodes[0].own = tree.nodes[0].own.saturating_add(child.bytes.unwrap_or(0));
            continue;
        }
        let name: Box<str> = child.name.as_str().into();
        if let Some(fresh) = walked.remove(&child.path) {
            tree.graft(0, name, &fresh, 0);
        } else if let (Some(old), Some(id)) =
            (stored.as_ref(), stored_ids.get(index).copied().flatten())
        {
            tree.graft(0, name, old, id);
        } else {
            // A folder with no total would leave the parent short. Keep the old tree instead.
            return;
        }
    }
    tree.roll_up();
    keep(&mut stored, tree);
}

/// A folder under the stored root replaces its old copy. Anything else becomes the new root.
fn keep(stored: &mut Option<Tree>, fresh: Tree) {
    if let Some(tree) = stored.as_mut()
        && fresh.root != tree.root
        && fresh.root.starts_with(&tree.root)
    {
        if let Some(id) = tree.find(&fresh.root) {
            tree.replace(id, &fresh);
        } else if let Some(parent) = fresh.root.parent().and_then(|parent| tree.find(parent)) {
            let name = fresh
                .root
                .file_name()
                .map(|name| name.to_string_lossy().into())
                .unwrap_or_default();
            tree.graft(parent, name, &fresh, 0);
            tree.roll_up();
        }
        return;
    }
    *stored = Some(fresh);
}

fn size_fields(size: Result<u64, MeasureError>) -> (u64, Option<String>) {
    match size {
        Ok(bytes) => (bytes, None),
        Err(err) => (0, Some(err.text())),
    }
}
