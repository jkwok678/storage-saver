//! Shared helpers for reading one folder.

use std::fs::FileType;

use super::types::ChildEntry;
use crate::platform::{self, Volume};

pub(crate) fn is_real_dir(file_type: &FileType) -> bool {
    file_type.is_dir() && !file_type.is_symlink()
}

/// One row per volume, for a top level that lists disks instead of a folder.
pub(crate) fn volume_listing() -> Vec<ChildEntry> {
    platform::volumes().iter().map(volume_entry).collect()
}

pub(crate) fn volume_entry(volume: &Volume) -> ChildEntry {
    ChildEntry {
        name: volume.label.clone(),
        path: volume.path.to_string_lossy().into_owned(),
        is_dir: true,
        bytes: Some(volume.total.saturating_sub(volume.available)),
        modified: None,
        error: volume.error.clone(),
    }
}
