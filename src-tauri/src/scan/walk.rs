//! Parallel walk that records every folder's total.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use super::listing::is_real_dir;
use super::lock;
use super::scan_cancelled;
use super::tree::{Tree, Trouble};
use crate::platform::{BulkEntry, FileKey, allocated_bytes, is_excluded, read_dir_bulk};

#[derive(Debug)]
pub(super) struct Cancelled;

/// Walk `path` and record the total of every folder under it.
pub(super) fn measure_tree(
    path: &Path,
    generation: Option<u64>,
    links: &Arc<Mutex<HashSet<FileKey>>>,
) -> Result<Tree, Cancelled> {
    if generation.is_some_and(scan_cancelled) {
        return Err(Cancelled);
    }

    let mut tree = Tree::new(path.to_path_buf());
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_real_dir(&metadata.file_type()) => {}
        Ok(metadata) => {
            if metadata.is_file() {
                tree.nodes[0].own = allocated_bytes(path, &metadata);
            }
            tree.roll_up();
            return Ok(tree);
        }
        Err(err) => {
            let mut trouble = Trouble::default();
            trouble.note_io(&err);
            tree.note(0, trouble);
            tree.roll_up();
            return Ok(tree);
        }
    }

    let walk = Walk {
        tree: Mutex::new(tree),
        links: links.clone(),
        generation,
    };
    walk_pool().scope(|scope| visit(scope, &walk, path.to_path_buf(), 0));
    if walk.cancelled() {
        return Err(Cancelled);
    }
    let mut tree = walk
        .tree
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tree.roll_up();
    Ok(tree)
}

/// What every folder read in one walk shares.
struct Walk {
    tree: Mutex<Tree>,
    links: Arc<Mutex<HashSet<FileKey>>>,
    generation: Option<u64>,
}

impl Walk {
    fn cancelled(&self) -> bool {
        self.generation.is_some_and(scan_cancelled)
    }
}

/// Runs on the walk pool once per folder. Adds the folder's files into its node, gives
/// each subfolder a node, and queues the subfolders as tasks of their own.
fn visit<'s>(scope: &rayon::Scope<'s>, walk: &'s Walk, dir: PathBuf, id: u32) {
    if walk.cancelled() {
        return;
    }

    let mut own = 0u64;
    let mut trouble = Trouble::default();
    let mut keyed = Vec::new();
    let mut folders = Vec::new();
    let read = read_dir_bulk(&dir, |entry| match entry {
        BulkEntry::File {
            bytes,
            key: Some(key),
            ..
        } => keyed.push((key, bytes)),
        BulkEntry::File { bytes, .. } => own = own.saturating_add(bytes),
        BulkEntry::Folder(name) => folders.push(name),
        BulkEntry::Other => {}
        BulkEntry::Error(errno) => trouble.note_io(&io::Error::from_raw_os_error(errno)),
    });
    if let Err(err) = read {
        trouble.note_io(&err);
    }
    folders.retain(|name| !is_excluded(&dir.join(name)));
    if !keyed.is_empty() {
        let mut set = lock(&walk.links);
        for (key, bytes) in keyed {
            if set.insert(key) {
                own = own.saturating_add(bytes);
            }
        }
    }

    let ids: Vec<u32> = {
        let mut tree = lock(&walk.tree);
        let node = &mut tree.nodes[id as usize];
        node.own = node.own.saturating_add(own);
        tree.note(id, trouble);
        folders
            .iter()
            .map(|name| tree.add_child(id, name.as_str().into()))
            .collect()
    };
    for (name, child) in folders.into_iter().zip(ids) {
        let path = dir.join(name);
        scope.spawn(move |scope| visit(scope, walk, path, child));
    }
}

pub(super) fn walk_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("storage-scan-{index}"))
            .build()
            .expect("storage scan thread pool")
    })
}

pub(super) fn worker_count(jobs: usize) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4);
    jobs.max(1).min(cpus)
}
