//! Everything that differs between operating systems. Shared code calls through here.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;

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
    Error(i32),
}

/// A common cleanup location under the user's home folder.
pub(crate) struct Notable {
    pub(crate) path: std::path::PathBuf,
    pub(crate) label: String,
    /// Worth measuring when no scan has covered it yet.
    pub(crate) measure: bool,
}

pub(crate) fn lowercase(path: &std::path::Path) -> std::path::PathBuf {
    std::path::PathBuf::from(path.to_string_lossy().to_lowercase())
}
