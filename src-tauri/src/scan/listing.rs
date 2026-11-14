//! Shared helpers for reading one folder.

use std::fs::FileType;

pub(crate) fn is_real_dir(file_type: &FileType) -> bool {
    file_type.is_dir() && !file_type.is_symlink()
}
