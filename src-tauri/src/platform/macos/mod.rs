//! macOS: `getattrlistbulk`, `statfs`, Finder and POSIX signals.

mod bulk;
mod trash;

use std::env;
use std::ffi::CString;
use std::fs::{self, Metadata};
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use super::{Notable, Volume};
use crate::scan::tree::{DENIED, MeasureError, Trouble};
use crate::scan::{ChildEntry, DiskSummary};

pub(crate) use bulk::read_dir_bulk;
pub(crate) use trash::{is_protected_path, move_to_trash, trash_dir_for};

/// Stored trees are kept per volume. macOS has one.
pub(crate) fn volume_slot(_path: &Path) -> usize {
    0
}

/// The path that lists every volume instead of a folder. macOS has a single root.
pub(crate) const VIRTUAL_ROOT: Option<&str> = None;

/// A path as sent by the frontend, in the form the rest of the backend expects.
pub(crate) fn normalize_input(path: &str) -> Result<String, String> {
    Ok(path.to_string())
}

pub(crate) fn is_volume_root(path: &Path) -> bool {
    path == Path::new("/")
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    env::var("HOME").ok().map(PathBuf::from)
}

pub(crate) fn volume_summary(_path: Option<&Path>) -> Result<DiskSummary, String> {
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    let path = CString::new("/").map_err(|err| err.to_string())?;
    // SAFETY: stat is initialized by statfs before it is read, and the path is a valid C string.
    let rc = unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: statfs returned success, so every field is initialized.
    let stat = unsafe { stat.assume_init() };
    let block = u64::from(stat.f_bsize);
    let total = stat.f_blocks.saturating_mul(block);
    let available = stat.f_bavail.saturating_mul(block);
    let used = total.saturating_sub(available);
    Ok(DiskSummary {
        total,
        available,
        used,
    })
}

pub(crate) fn volumes() -> Vec<Volume> {
    let (total, available, error) = match volume_summary(None) {
        Ok(summary) => (summary.total, summary.available, None),
        Err(err) => (0, 0, Some(err)),
    };
    vec![Volume {
        path: PathBuf::from("/"),
        label: "Boot Disk".to_string(),
        total,
        available,
        error,
    }]
}

pub(crate) fn list_children(root: &Path) -> Result<Vec<ChildEntry>, String> {
    let entries = fs::read_dir(root).map_err(|err| {
        if err.kind() == ErrorKind::PermissionDenied {
            "Not permitted".to_string()
        } else {
            err.to_string()
        }
    })?;

    let mut children = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if is_excluded(&path) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(_) => match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata.file_type(),
                Err(_) => continue,
            },
        };
        let is_dir = crate::scan::listing::is_real_dir(&file_type);
        let metadata = fs::symlink_metadata(&path);
        let modified = metadata.as_ref().ok().and_then(modified_secs);
        let (bytes, error) = if is_dir {
            (None, None)
        } else if let Ok(metadata) = &metadata {
            // Links, sockets and the like take no space of their own worth showing.
            let bytes = if file_type.is_file() {
                allocated_bytes(&path, metadata)
            } else {
                0
            };
            (Some(bytes), None)
        } else if !file_type.is_file() {
            (Some(0), None)
        } else {
            let mut trouble = Trouble::default();
            if let Err(err) = &metadata {
                trouble.note_io(err);
            }
            if trouble.flags & DENIED != 0 {
                (None, Some(MeasureError::NotPermitted.text()))
            } else {
                (Some(0), trouble.message)
            }
        };
        children.push(ChildEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            is_dir,
            bytes,
            modified,
            error,
        });
    }
    Ok(children)
}

/// Other mounts that would count the same files twice, or that are not this boot disk.
/// `/System/Volumes` repeats `/Users`, `/Applications`, and the rest through APFS firmlinks.
pub(crate) fn is_excluded(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor == Path::new("/System/Volumes")
            || ancestor == Path::new("/Volumes")
            || ancestor == Path::new("/dev")
            || ancestor == Path::new("/.vol")
    })
}

/// Allocated space, in bytes. `st_blocks` is reported in 512-byte units.
pub(crate) fn allocated_bytes(_path: &Path, metadata: &Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

fn modified_secs(metadata: &Metadata) -> Option<u64> {
    u64::try_from(metadata.mtime()).ok()
}

/// Cleanup locations under the home folder.
pub(crate) fn notable_locations() -> Vec<Notable> {
    let Some(home) = home_dir() else {
        return Vec::new();
    };
    [
        "Library/Caches",
        "Downloads",
        ".Trash",
        "Library/Logs",
        "Library/Developer/Xcode/DerivedData",
        "Library/Developer/CoreSimulator",
        "Library/Application Support",
        "Library/Containers",
        ".npm",
        ".cargo/registry",
        ".cache",
    ]
    .into_iter()
    .map(|rel| Notable {
        path: home.join(rel),
        label: format!("~/{rel}"),
        measure: matches!(
            rel,
            ".Trash"
                | "Library/Caches"
                | "Library/Logs"
                | "Library/Developer/Xcode/DerivedData"
                | ".npm"
                | ".cargo/registry"
                | ".cache"
        ),
    })
    .collect()
}

/// Where the Claude and Cursor CLIs usually live, ahead of `PATH`.
pub(crate) fn cli_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home_dir() {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".claude/local"));
        dirs.push(home.join(".cursor/bin"));
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        dirs.push(PathBuf::from(dir));
    }
    dirs
}

/// Suffixes tried after a CLI's name when looking it up.
pub(crate) const CLI_EXTENSIONS: &[&str] = &[""];

pub(crate) fn prepare_command(_cmd: &mut Command) {}

/// What it takes to stop a running child later.
#[derive(Clone, PartialEq)]
pub(crate) struct ChildHandle(u32);

pub(crate) fn track_child(child: &Child) -> ChildHandle {
    ChildHandle(child.id())
}

pub(crate) fn terminate(handle: &ChildHandle) {
    // SAFETY: Sending SIGTERM to the tracked child process ID. The handle is cleared before the
    // child is reaped, so it still names our child here.
    unsafe {
        libc::kill(handle.0 as i32, libc::SIGTERM);
    }
}

/// The machine, as the advisor names it to the user.
pub(crate) const MACHINE: &str = "Mac";

/// The advisor's opening instructions, up to the scanned data.
pub(crate) const ADVISOR_RULES: &str = "You are the Storage Saver AI Advisor for macOS. Your job is to help the user understand what is taking up disk space on their Mac and identify what they can and should delete.\n\n\
         Rules:\n\
         1. Base your analysis on the scanned storage data below, and use your read-only file/directory tools if you need to inspect specific directories further.\n\
         2. Never attempt to delete, modify, or move files yourself. The user will review and move items to Trash using Storage Saver's UI.\n\
         3. Clearly distinguish between:\n\
            - `safe`: Safe to delete anytime without losing personal data (e.g. ~/Library/Caches/*, Xcode DerivedData, npm/cargo/homebrew caches, old logs, ~/.Trash).\n\
            - `caution`: Worth reviewing before deleting (e.g. large files in ~/Downloads, old .dmg/.zip installers, Docker/VM images, old project node_modules/target build folders, large videos).\n\
            - System/protected paths (/System, /Library, active app bundles) that should NOT be deleted.\n\
         4. Mention concrete file/folder paths and their human-readable sizes in your explanation.\n\
         5. IMPORTANT: Whenever you recommend specific files or folders that the user can delete or review for deletion, include a fenced `recommendations` JSON block at the VERY END of your response in this exact format:\n\
         ```recommendations\n\
         [\n\
           {\n\
             \"path\": \"/Users/example/Library/Caches/com.example.app\",\n\
             \"safety\": \"safe\",\n\
             \"reason\": \"App cache that will be regenerated automatically if needed.\"\n\
           }\n\
         ]\n\
         ```\n\
         Only include real, absolute paths on this Mac in the `recommendations` block.\n\n\
         === SCANNED STORAGE DATA ===\n";
