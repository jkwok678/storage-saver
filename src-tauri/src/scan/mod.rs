//! Folder sizes, largest files, and moving items to the Trash.
//!
//! Finished scans keep their totals here. Listing a folder, ranking files, and the advisor
//! snapshot read those totals instead of walking the disk again.

mod bulk;
mod directory;
mod largest;
mod listing;
mod snapshot;
mod trash;
mod tree;
mod types;
mod walk;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use tauri::ipc::Channel;

pub use snapshot::{inspect_path_size, storage_snapshot};
pub use trash::{is_protected_path, trash_path_into};
pub use tree::Tree;
pub use types::{
    ChildEntry, DiskSummary, LargestEvent, LargestFile, ScanEvent, SnapshotItem, StorageSnapshot,
    TrashOutcome,
};

/// Latest scan wins. Older walks notice the generation change and stop.
static SCAN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Latest largest-files scan wins. Kept apart from `SCAN_GENERATION` so browsing and
/// ranking can be stopped independently when the view changes.
static LARGEST_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Folder totals from finished scans. Opening a folder reads them instead of walking it again.
static STORED: Mutex<Option<Tree>> = Mutex::new(None);

/// Latest ranked largest files from the Largest Files scan, shared with AI context.
static STORED_LARGEST: Mutex<Vec<LargestFile>> = Mutex::new(Vec::new());

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn scan_cancelled(generation: u64) -> bool {
    SCAN_GENERATION.load(Ordering::Acquire) != generation
}

fn cancel_if_current(generation: u64) {
    let _ = SCAN_GENERATION.compare_exchange(
        generation,
        generation.wrapping_add(1),
        Ordering::AcqRel,
        Ordering::Acquire,
    );
}

fn largest_cancelled(generation: u64) -> bool {
    LARGEST_GENERATION.load(Ordering::Acquire) != generation
}

#[tauri::command]
pub fn disk_summary() -> Result<DiskSummary, String> {
    listing::read_disk_summary()
}

#[tauri::command]
pub async fn scan_directory(
    path: String,
    refresh: bool,
    on_event: Channel<ScanEvent>,
) -> Result<(), String> {
    directory::scan_directory(path, refresh, on_event).await
}

#[tauri::command]
pub fn stop_scan() {
    SCAN_GENERATION.fetch_add(1, Ordering::AcqRel);
}

#[tauri::command]
pub async fn scan_largest_files(
    path: String,
    limit: usize,
    on_event: Channel<LargestEvent>,
) -> Result<(), String> {
    largest::scan_largest_files(path, limit, on_event).await
}

#[tauri::command]
pub fn stop_largest_scan() {
    LARGEST_GENERATION.fetch_add(1, Ordering::AcqRel);
}

#[tauri::command]
pub async fn move_to_trash(path: String) -> Result<TrashOutcome, String> {
    trash::move_to_trash(path).await
}

#[cfg(test)]
mod tests;
