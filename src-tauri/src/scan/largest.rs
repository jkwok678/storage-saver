//! Rank the largest files under a folder, counting each hard link once.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::ipc::Channel;

use super::listing::is_real_dir;
use super::lock;
use super::types::{LargestEvent, LargestFile};
use super::walk::walk_pool;
use super::{LARGEST_GENERATION, STORED_LARGEST, largest_cancelled};
use crate::platform::{BulkEntry, FileKey, is_excluded, read_dir_bulk};

pub(super) async fn scan_largest_files(
    path: String,
    limit: usize,
    on_event: Channel<LargestEvent>,
) -> Result<(), String> {
    match tauri::async_runtime::spawn_blocking(move || run_largest_scan(path, limit, on_event))
        .await
    {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

fn run_largest_scan(
    path: String,
    limit: usize,
    on_event: Channel<LargestEvent>,
) -> Result<(), String> {
    let generation = LARGEST_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let root = PathBuf::from(&path);
    if !root.is_absolute() {
        return Err("path must be absolute".into());
    }
    let metadata = fs::symlink_metadata(&root).map_err(|err| err.to_string())?;
    if !is_real_dir(&metadata.file_type()) {
        return Err("path must be a folder".into());
    }
    let scan = LargestScan::new(limit.clamp(1, 1_000));

    std::thread::scope(|scope| {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let events = &on_event;
        let scan_ref = &scan;
        scope.spawn(move || {
            let mut last_scanned = u64::MAX;
            loop {
                match done_rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if largest_cancelled(generation) {
                            break;
                        }
                        let scanned = scan_ref.scanned.load(Ordering::Relaxed);
                        if scanned == last_scanned {
                            continue;
                        }
                        last_scanned = scanned;
                        let files = scan_ref.snapshot();
                        *lock(&STORED_LARGEST) = files.clone();
                        if events.send(LargestEvent::Batch { files, scanned }).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        walk_pool().scope(|walk_scope| {
            largest_walk(walk_scope, &scan, root.clone(), Some(generation));
        });
        drop(done_tx);
    });

    if largest_cancelled(generation) {
        return Ok(());
    }
    let scanned = scan.scanned.load(Ordering::Relaxed);
    let final_files = scan.snapshot();
    *lock(&STORED_LARGEST) = final_files.clone();
    let _ = on_event.send(LargestEvent::Batch {
        files: final_files,
        scanned,
    });
    let _ = on_event.send(LargestEvent::Done { scanned });
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RankedFile {
    bytes: u64,
    path: String,
    name: String,
}

impl Ord for RankedFile {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.bytes
            .cmp(&other.bytes)
            .then_with(|| self.path.cmp(&other.path))
    }
}

impl PartialOrd for RankedFile {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The largest files seen so far, kept as a min-heap so only `limit` are held at once.
pub(super) struct LargestScan {
    limit: usize,
    files: Mutex<BinaryHeap<Reverse<RankedFile>>>,
    /// Hard links count once, as they do in folder sizes.
    links: Mutex<HashSet<FileKey>>,
    scanned: AtomicU64,
}

impl LargestScan {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            limit,
            files: Mutex::new(BinaryHeap::with_capacity(limit)),
            links: Mutex::new(HashSet::new()),
            scanned: AtomicU64::new(0),
        }
    }

    fn add(&self, dir: &Path, files: Vec<(String, u64)>) {
        if files.is_empty() {
            return;
        }
        let mut heap = lock(&self.files);
        for (name, bytes) in files {
            if heap.len() >= self.limit {
                let Some(smallest) = heap.peek().map(|file| file.0.bytes) else {
                    continue;
                };
                if bytes <= smallest {
                    continue;
                }
            }
            heap.push(Reverse(RankedFile {
                bytes,
                path: dir.join(&name).to_string_lossy().into_owned(),
                name,
            }));
            if heap.len() > self.limit {
                heap.pop();
            }
        }
    }

    pub(super) fn snapshot(&self) -> Vec<LargestFile> {
        let heap = lock(&self.files);
        let mut files: Vec<&RankedFile> = heap.iter().map(|file| &file.0).collect();
        files.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        files
            .into_iter()
            .map(|file| LargestFile {
                name: file.name.clone(),
                path: file.path.clone(),
                bytes: file.bytes,
            })
            .collect()
    }
}

/// Walk `dir` and keep its files in `scan`, queueing subfolders as tasks of their own.
pub(super) fn largest_walk<'s>(
    scope: &rayon::Scope<'s>,
    scan: &'s LargestScan,
    dir: PathBuf,
    generation: Option<u64>,
) {
    if generation.is_some_and(largest_cancelled) {
        return;
    }

    let mut files = Vec::new();
    let mut keyed = Vec::new();
    let mut folders = Vec::new();
    // An unreadable folder simply contributes no files to the ranking.
    let _ = read_dir_bulk(&dir, |entry| match entry {
        BulkEntry::File {
            name,
            bytes,
            key: Some(key),
        } => keyed.push((key, name, bytes)),
        BulkEntry::File { name, bytes, .. } => files.push((name, bytes)),
        BulkEntry::Folder(name) => folders.push(name),
        BulkEntry::Other | BulkEntry::Error(_) => {}
    });
    folders.retain(|name| !is_excluded(&dir.join(name)));
    if !keyed.is_empty() {
        let mut set = lock(&scan.links);
        for (key, name, bytes) in keyed {
            if set.insert(key) {
                files.push((name, bytes));
            }
        }
    }

    // Count only after ranking, so the reporter never snapshots a count ahead of the heap.
    let count = files.len() as u64;
    scan.add(&dir, files);
    scan.scanned.fetch_add(count, Ordering::Relaxed);
    for name in folders {
        let path = dir.join(name);
        scope.spawn(move |scope| largest_walk(scope, scan, path, generation));
    }
}
