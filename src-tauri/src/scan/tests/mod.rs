use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64 as StdAtomic;

use super::directory::scan_into;
use super::largest::{LargestScan, largest_walk};
use super::snapshot::ranked_folders;
use super::trash::move_into_trash;
use super::tree::MeasureError;
use super::walk::{measure_tree, walk_pool};
use super::*;
use crate::platform::allocated_bytes;

mod shared;

#[cfg(target_os = "macos")]
mod macos;

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "storage-saver-{}-{}",
            std::process::id(),
            next_temp_id()
        ));
        fs::create_dir_all(&path).unwrap();
        // The temp dir can sit behind a symlink (/var -> /private/var). Stored paths are compared as given.
        Self(dunce::canonicalize(&path).unwrap())
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn next_temp_id() -> u64 {
    static NEXT: StdAtomic = StdAtomic::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn file_bytes(path: &Path) -> u64 {
    allocated_bytes(path, &fs::symlink_metadata(path).unwrap())
}

fn measure(path: &Path) -> Result<u64, MeasureError> {
    measure_tree(path, None, &Arc::default())
        .unwrap()
        .size_of(0)
}

fn largest_in(root: &Path, limit: usize) -> Vec<LargestFile> {
    let scan = LargestScan::new(limit);
    walk_pool().scope(|scope| largest_walk(scope, &scan, root.to_path_buf(), None));
    scan.snapshot()
}

/// Runs a scan against `stored` and returns the listing with every size filled in,
/// plus the folders that were walked rather than read from the stored tree.
fn scan(stored: &Mutex<Option<Tree>>, dir: &Path, refresh: bool) -> (Vec<ChildEntry>, Vec<String>) {
    let mut entries = Vec::new();
    let mut walked = Vec::new();
    let mut done = false;
    scan_into(stored, dir, refresh, None, |event| {
        match event {
            ScanEvent::Started { entries: listed } => entries = listed,
            ScanEvent::Sized { path, bytes, error } => {
                let entry = entries.iter_mut().find(|entry| entry.path == path).unwrap();
                entry.bytes = Some(bytes);
                entry.error = error;
                walked.push(path);
            }
            ScanEvent::Done => done = true,
        }
        true
    })
    .unwrap();
    assert!(done);
    walked.sort();
    (entries, walked)
}

fn bytes_of(entries: &[ChildEntry], name: &str) -> u64 {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .and_then(|entry| entry.bytes)
        .unwrap()
}

fn path_of(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// Tests move into their own folder, never the real Trash through Finder.
fn move_plainly(target: &Path, trash_dir: &Path) -> Result<(PathBuf, bool), String> {
    move_into_trash(target, trash_dir).map(|dest| (dest, false))
}
