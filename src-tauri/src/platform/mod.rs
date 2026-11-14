//! Everything that differs between operating systems. Shared code calls through here.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(any(windows, test))]
pub(crate) mod winpath;

#[cfg(not(any(target_os = "macos", windows)))]
compile_error!("Storage Saver supports macOS and Windows only.");

/// Identifies a file that other names may link to: volume and file id.
pub(crate) type FileKey = (u64, u64);

/// One entry from a bulk directory read.
pub(crate) enum BulkEntry {
    File {
        name: String,
        bytes: u64,
        /// Set only for files that can be linked from elsewhere, so they count once.
        key: Option<FileKey>,
    },
    Folder(String),
    /// Links, sockets and the like. They take no space of their own worth counting.
    Other,
    /// Only macOS reports an error per entry. Windows reports it for the whole folder.
    #[cfg_attr(windows, allow(dead_code))]
    Error(i32),
}

/// A common cleanup location under the user's home folder.
pub(crate) struct Notable {
    pub(crate) path: std::path::PathBuf,
    pub(crate) label: String,
    /// Worth measuring when no scan has covered it yet.
    pub(crate) measure: bool,
}

/// A disk the user can open from the top level.
pub(crate) struct Volume {
    pub(crate) path: std::path::PathBuf,
    pub(crate) label: String,
    pub(crate) total: u64,
    pub(crate) available: u64,
    /// Why its capacity could not be read.
    pub(crate) error: Option<String>,
}

pub(crate) fn lowercase(path: &std::path::Path) -> std::path::PathBuf {
    std::path::PathBuf::from(path.to_string_lossy().to_lowercase())
}
