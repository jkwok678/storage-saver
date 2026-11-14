//! Windows: `GetFileInformationByHandleEx`, drive letters, the Recycle Bin and job objects.

mod bulk;
mod process;
mod recycle;
mod volumes;

use std::env;
use std::fs::Metadata;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use windows_sys::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_HIDDEN, FILE_ATTRIBUTE_SYSTEM};

use super::{Notable, winpath};
use crate::scan::ChildEntry;
use crate::scan::tree::MeasureError;

pub(crate) use bulk::read_dir_bulk;
pub(crate) use process::{ChildHandle, prepare_command, terminate, track_child};
pub(crate) use recycle::{is_protected_path, move_to_trash, trash_dir_for};
pub(crate) use volumes::{volume_summary, volumes};

/// The path that lists every volume instead of a folder.
pub(crate) const VIRTUAL_ROOT: Option<&str> = Some("");

/// A path as sent by the frontend, in the form the rest of the backend expects. The top level
/// stays empty.
pub(crate) fn normalize_input(path: &str) -> Result<String, String> {
    if path.trim().is_empty() {
        return Ok(String::new());
    }
    winpath::normalize(path).ok_or_else(|| format!("Unsupported path: {path}"))
}

/// Stored trees are kept per volume: one for each drive letter, and 0 for anything else.
pub(crate) fn volume_slot(path: &Path) -> usize {
    path.to_str()
        .and_then(winpath::drive_index)
        .map_or(0, |drive| usize::from(drive) + 1)
}

pub(crate) fn is_volume_root(path: &Path) -> bool {
    path.to_str().is_some_and(winpath::is_drive_root)
}

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    env_path("USERPROFILE")
}

/// List `root` from the same enumeration the walks use, so the rows add up to the walk totals.
pub(crate) fn list_children(root: &Path) -> Result<Vec<ChildEntry>, String> {
    let mut children = Vec::new();
    bulk::enumerate(root, |entry, _| {
        let hidden_link = entry.is_link()
            && entry.attributes & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)
                == FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM;
        if hidden_link {
            return;
        }
        // Links and folders that would download when opened take no space worth showing.
        let is_dir = entry.is_dir() && !entry.is_link() && !entry.is_recall_on_open();
        let bytes = if is_dir {
            None
        } else if entry.is_dir() || entry.is_link() {
            Some(0)
        } else {
            Some(entry.allocated)
        };
        children.push(ChildEntry {
            name: entry.name.clone(),
            path: root.join(&entry.name).to_string_lossy().into_owned(),
            is_dir,
            bytes,
            modified: winpath::filetime_to_unix(entry.written),
            error: None,
        });
    })
    .map_err(|err| {
        if err.kind() == ErrorKind::PermissionDenied {
            MeasureError::NotPermitted.text()
        } else {
            err.to_string()
        }
    })?;
    Ok(children)
}

/// Windows has no mounts that would count the same files twice.
pub(crate) fn is_excluded(_path: &Path) -> bool {
    false
}

/// Allocated space, in bytes, read from the file system. Falls back to the file's length.
pub(crate) fn allocated_bytes(path: &Path, metadata: &Metadata) -> u64 {
    bulk::allocated_size(path).unwrap_or(metadata.len())
}

/// Cleanup locations: the environment variable that holds the base, the folder below it, and
/// whether it is worth measuring when no scan has covered it yet.
const NOTABLE: &[(&str, &str, bool)] = &[
    ("LOCALAPPDATA", "Temp", true),
    ("USERPROFILE", "Downloads", false),
    ("LOCALAPPDATA", "Microsoft\\Windows\\INetCache", true),
    ("LOCALAPPDATA", "CrashDumps", true),
    ("LOCALAPPDATA", "npm-cache", true),
    ("LOCALAPPDATA", "pip\\Cache", true),
    ("USERPROFILE", ".cargo\\registry", true),
    ("USERPROFILE", ".nuget\\packages", true),
    ("LOCALAPPDATA", "Packages", false),
    ("SystemRoot", "Temp", false),
    ("SystemRoot", "SoftwareDistribution\\Download", false),
];

/// Cleanup locations under the profile and the Windows folder.
pub(crate) fn notable_locations() -> Vec<Notable> {
    NOTABLE
        .iter()
        .filter_map(|&(var, rel, measure)| {
            Some(Notable {
                path: env_path(var)?.join(rel),
                label: format!("%{var}%\\{rel}"),
                measure,
            })
        })
        .collect()
}

/// Where the Claude and Cursor CLIs usually live, ahead of `PATH`.
pub(crate) fn cli_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = home_dir() {
        dirs.push(home.join(".local\\bin"));
    }
    if let Some(appdata) = env_path("APPDATA") {
        dirs.push(appdata.join("npm"));
    }
    if let Some(local) = env_path("LOCALAPPDATA") {
        dirs.push(local.join("cursor-agent"));
        dirs.push(local.join("Programs\\cursor-agent"));
    }
    dirs
}

/// Suffixes tried after a CLI's name when looking it up.
pub(crate) const CLI_EXTENSIONS: &[&str] = &[".exe", ".cmd", ".bat"];

/// The machine, as the advisor names it to the user.
pub(crate) const MACHINE: &str = "PC";

/// The advisor's opening instructions, up to the scanned data.
pub(crate) const ADVISOR_RULES: &str = "You are the Storage Saver AI Advisor for Windows. Your job is to help the user understand what is taking up disk space on their PC and identify what they can and should delete.\n\n\
         Rules:\n\
         1. Base your analysis on the scanned storage data below, and use your read-only file/directory tools if you need to inspect specific directories further.\n\
         2. Never attempt to delete, modify, or move files yourself. The user will review and move items to the Recycle Bin using Storage Saver's UI.\n\
         3. Clearly distinguish between:\n\
            - `safe`: Safe to delete anytime without losing personal data (e.g. %LOCALAPPDATA%\\Temp, browser caches, npm/pip/cargo/nuget caches, C:\\Windows\\SoftwareDistribution\\Download, crash dumps, the Recycle Bin).\n\
            - `caution`: Worth reviewing before deleting (e.g. large files in Downloads, old .exe/.msi/.zip installers, WSL/Docker/VM disk images (.vhdx), old project node_modules/target build folders, large videos).\n\
            - System/protected paths (C:\\Windows, C:\\Program Files, C:\\ProgramData, pagefile.sys and hiberfil.sys) that should NOT be deleted.\n\
         4. Mention concrete file/folder paths and their human-readable sizes in your explanation.\n\
         5. IMPORTANT: Whenever you recommend specific files or folders that the user can delete or review for deletion, include a fenced `recommendations` JSON block at the VERY END of your response in this exact format:\n\
         ```recommendations\n\
         [\n\
           {\n\
             \"path\": \"C:/Users/example/AppData/Local/Temp/example\",\n\
             \"safety\": \"safe\",\n\
             \"reason\": \"App cache that will be regenerated automatically if needed.\"\n\
           }\n\
         ]\n\
         ```\n\
         Only include real, absolute paths on this PC in the `recommendations` block. In the recommendations block write paths with forward slashes, e.g. C:/Users/you/Downloads/file.zip, because backslashes are not valid in JSON strings.\n\n\
         === SCANNED STORAGE DATA ===\n";
