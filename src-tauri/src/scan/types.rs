//! Sizes, listings, and scan events sent to the frontend.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSummary {
    pub total: u64,
    pub available: u64,
    pub used: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildEntry {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) is_dir: bool,
    /// Known up front for files and stored folders. `None` until a walk measures the folder.
    pub(crate) bytes: Option<u64>,
    /// Last modification time, in seconds since the Unix epoch.
    pub(crate) modified: Option<u64>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum ScanEvent {
    Started {
        entries: Vec<ChildEntry>,
    },
    Sized {
        path: String,
        bytes: u64,
        error: Option<String>,
    },
    Done,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargestFile {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum LargestEvent {
    Batch {
        files: Vec<LargestFile>,
        scanned: u64,
    },
    Done {
        scanned: u64,
    },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotItem {
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
    pub own_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageSnapshot {
    pub disk: Option<DiskSummary>,
    pub current_path: Option<String>,
    pub current_items: Vec<SnapshotItem>,
    pub top_folders: Vec<SnapshotItem>,
    pub dense_folders: Vec<SnapshotItem>,
    pub largest_files: Vec<LargestFile>,
    pub notable_locations: Vec<SnapshotItem>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashOutcome {
    pub original_path: String,
    pub trash_path: String,
    pub freed_bytes: u64,
    /// Whether Finder trashed it, so "Put Back" can restore it.
    pub put_back: bool,
}
