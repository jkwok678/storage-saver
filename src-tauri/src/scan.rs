use jwalk::{Parallelism, WalkDir};
use serde::Serialize;
use std::collections::HashSet;
use std::ffi::CString;
use std::fs::{self, FileType};
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::ipc::Channel;

/// Latest scan wins. Older walks notice the generation change and stop.
static SCAN_GENERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskSummary {
    pub total: u64,
    pub available: u64,
    pub used: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildEntry {
    name: String,
    path: String,
    is_dir: bool,
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

#[derive(Debug)]
enum MeasureError {
    NotPermitted,
    Cancelled,
    Message(String),
}

#[tauri::command]
pub fn disk_summary() -> Result<DiskSummary, String> {
    read_disk_summary()
}

#[tauri::command]
pub async fn scan_directory(path: String, on_event: Channel<ScanEvent>) -> Result<(), String> {
    // The walk is long. Run it off the UI thread so folder names and sizes can paint as they arrive.
    match tauri::async_runtime::spawn_blocking(move || run_scan(path, on_event)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

fn run_scan(path: String, on_event: Channel<ScanEvent>) -> Result<(), String> {
    let generation = SCAN_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let root = PathBuf::from(&path);
    if !root.is_absolute() {
        return Err("path must be absolute".into());
    }

    let mut children = list_children(&root)?;
    children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    if scan_cancelled(generation) {
        return Ok(());
    }
    if on_event
        .send(ScanEvent::Started {
            entries: children.clone(),
        })
        .is_err()
    {
        cancel_if_current(generation);
        return Ok(());
    }

    if children.is_empty() {
        if on_event.send(ScanEvent::Done).is_err() {
            cancel_if_current(generation);
        }
        return Ok(());
    }

    let mut completed = true;
    let queue = Mutex::new(children);
    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        let workers = worker_count(queue.lock().map(|jobs| jobs.len()).unwrap_or(1));
        let queue = &queue;
        for _ in 0..workers {
            let tx = tx.clone();
            scope.spawn(move || {
                loop {
                    if scan_cancelled(generation) {
                        break;
                    }
                    let job = {
                        let mut jobs = match queue.lock() {
                            Ok(jobs) => jobs,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        jobs.pop()
                    };
                    let Some(job) = job else { break };
                    let measured = measure_path(&PathBuf::from(&job.path), generation);
                    if tx.send((job.path, measured)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        while let Ok((path, measured)) = rx.recv() {
            if scan_cancelled(generation) {
                completed = false;
                break;
            }
            let (bytes, error) = match measured {
                Ok(bytes) => (bytes, None),
                Err(MeasureError::NotPermitted) => (0, Some("Not permitted".to_string())),
                Err(MeasureError::Cancelled) => {
                    completed = false;
                    break;
                }
                Err(MeasureError::Message(message)) => (0, Some(message)),
            };
            if on_event
                .send(ScanEvent::Sized { path, bytes, error })
                .is_err()
            {
                cancel_if_current(generation);
                completed = false;
                break;
            }
        }
    });

    if completed && !scan_cancelled(generation) && on_event.send(ScanEvent::Done).is_err() {
        cancel_if_current(generation);
    }
    Ok(())
}

fn read_disk_summary() -> Result<DiskSummary, String> {
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

fn list_children(root: &Path) -> Result<Vec<ChildEntry>, String> {
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
        children.push(ChildEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            is_dir: is_real_dir(&file_type),
        });
    }
    Ok(children)
}

/// Other mounts that would count the same files twice, or that are not this boot disk.
/// `/System/Volumes` repeats `/Users`, `/Applications`, and the rest through APFS firmlinks.
fn is_excluded(path: &Path) -> bool {
    path.ancestors().any(|ancestor| {
        ancestor == Path::new("/System/Volumes")
            || ancestor == Path::new("/Volumes")
            || ancestor == Path::new("/dev")
            || ancestor == Path::new("/.vol")
    })
}

fn is_real_dir(file_type: &FileType) -> bool {
    file_type.is_dir() && !file_type.is_symlink()
}

fn measure_path(path: &Path, generation: u64) -> Result<u64, MeasureError> {
    measure_path_inner(path, Some(generation))
}

fn measure_path_inner(path: &Path, generation: Option<u64>) -> Result<u64, MeasureError> {
    if generation.is_some_and(scan_cancelled) {
        return Err(MeasureError::Cancelled);
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            return Err(MeasureError::NotPermitted);
        }
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(MeasureError::Message(err.to_string())),
    };

    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        if metadata.is_file() {
            return Ok(allocated_bytes(&metadata));
        }
        return Ok(0);
    }

    // jwalk records an unreadable root on an entry the size loop never sees.
    if let Err(err) = fs::read_dir(path) {
        return match err.kind() {
            ErrorKind::PermissionDenied => Err(MeasureError::NotPermitted),
            ErrorKind::NotFound => Ok(0),
            _ => Err(MeasureError::Message(err.to_string())),
        };
    }

    let mut total = 0u64;
    let mut saw_permission = false;
    let mut other_error: Option<String> = None;
    let mut seen_links: HashSet<(u64, u64)> = HashSet::new();
    let walk_generation = generation;

    let walk = WalkDir::new(path)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(Parallelism::RayonExistingPool {
            pool: walk_pool(),
            // Callers wait on std threads, not this pool, so a busy pool should queue
            // instead of aborting the walk.
            busy_timeout: None,
        })
        .process_read_dir(move |_depth, _dir, _state, children| {
            if walk_generation.is_some_and(scan_cancelled) {
                children.clear();
                return;
            }
            for child in children.iter_mut() {
                if let Ok(entry) = child {
                    if is_excluded(&entry.path()) {
                        entry.read_children = None;
                    }
                }
            }
        });

    for entry in walk {
        if generation.is_some_and(scan_cancelled) {
            return Err(MeasureError::Cancelled);
        }
        match entry {
            Ok(entry) => {
                if let Some(children) = &entry.read_children {
                    if let Some(err) = children.error() {
                        note_walk_error(err, &mut saw_permission, &mut other_error);
                    }
                }
                let file_type = entry.file_type();
                if !file_type.is_file() || file_type.is_symlink() {
                    continue;
                }
                match entry.metadata() {
                    Ok(metadata) => {
                        if metadata.nlink() > 1
                            && !seen_links.insert((metadata.dev(), metadata.ino()))
                        {
                            continue;
                        }
                        total = total.saturating_add(allocated_bytes(&metadata));
                    }
                    Err(err) => note_walk_error(&err, &mut saw_permission, &mut other_error),
                }
            }
            Err(err) => note_walk_error(&err, &mut saw_permission, &mut other_error),
        }
    }

    if saw_permission && total == 0 {
        return Err(MeasureError::NotPermitted);
    }
    if total == 0 {
        if let Some(message) = other_error {
            return Err(MeasureError::Message(message));
        }
    }
    Ok(total)
}

fn note_walk_error(
    err: &jwalk::Error,
    saw_permission: &mut bool,
    other_error: &mut Option<String>,
) {
    let Some(io_error) = err.io_error() else {
        if other_error.is_none() {
            *other_error = Some(err.to_string());
        }
        return;
    };
    match io_error.kind() {
        ErrorKind::PermissionDenied => *saw_permission = true,
        ErrorKind::NotFound => {}
        _ => {
            if other_error.is_none() {
                *other_error = Some(io_error.to_string());
            }
        }
    }
}

/// Allocated space, in bytes. `st_blocks` is reported in 512-byte units.
fn allocated_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

fn walk_pool() -> Arc<jwalk::rayon::ThreadPool> {
    static POOL: OnceLock<Arc<jwalk::rayon::ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4);
        Arc::new(
            jwalk::rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .thread_name(|index| format!("storage-scan-{index}"))
                .build()
                .expect("storage scan thread pool"),
        )
    })
    .clone()
}

fn worker_count(jobs: usize) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4);
    jobs.max(1).min(cpus)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::sync::atomic::AtomicU64 as StdAtomic;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "storage-saver-{}-{}",
                std::process::id(),
                next_temp_id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
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
        allocated_bytes(&fs::symlink_metadata(path).unwrap())
    }

    #[test]
    fn disk_summary_matches_boot_volume_capacity() {
        let summary = read_disk_summary().unwrap();
        let output = std::process::Command::new("df")
            .args(["-k", "/"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let blocks: u64 = text
            .lines()
            .nth(1)
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(summary.total, blocks.saturating_mul(1024));
        assert_eq!(summary.used + summary.available, summary.total);
        assert!(summary.used > 0);
    }

    #[test]
    fn exclusion_skips_duplicate_and_other_mounts_only() {
        assert!(is_excluded(Path::new("/System/Volumes")));
        assert!(is_excluded(Path::new("/System/Volumes/Data/Users")));
        assert!(is_excluded(Path::new("/Volumes")));
        assert!(is_excluded(Path::new("/Volumes/Backup")));
        assert!(is_excluded(Path::new("/dev")));
        assert!(is_excluded(Path::new("/.vol")));
        assert!(!is_excluded(Path::new("/")));
        assert!(!is_excluded(Path::new("/System")));
        assert!(!is_excluded(Path::new("/Users")));
        assert!(!is_excluded(Path::new("/VolumesExtra")));
    }

    #[test]
    fn measure_counts_hidden_files_once_and_does_not_follow_links() {
        let root = TempDir::new();
        let file = root.0.join("a.txt");
        fs::write(&file, vec![1u8; 1024 * 1024]).unwrap();
        fs::hard_link(&file, root.0.join("b.txt")).unwrap();
        symlink("a.txt", root.0.join("link.txt")).unwrap();
        symlink("/usr", root.0.join("usr-link")).unwrap();
        let hidden = root.0.join(".hidden");
        fs::write(&hidden, vec![2u8; 128 * 1024]).unwrap();
        let nested = root.0.join("sub");
        fs::create_dir(&nested).unwrap();
        let nested_file = nested.join("c.txt");
        fs::write(&nested_file, vec![3u8; 64 * 1024]).unwrap();

        let measured = measure_path_inner(&root.0, None).unwrap();
        let expected = file_bytes(&file) + file_bytes(&hidden) + file_bytes(&nested_file);
        assert!(
            expected > 500_000,
            "expected allocated bytes, got {expected}"
        );
        assert_eq!(measured, expected);

        let listed = list_children(&root.0).unwrap();
        assert!(
            listed
                .iter()
                .any(|entry| entry.name == "sub" && entry.is_dir)
        );
        assert!(
            listed
                .iter()
                .any(|entry| entry.name == "a.txt" && !entry.is_dir)
        );
        assert!(
            listed
                .iter()
                .any(|entry| entry.name == "usr-link" && !entry.is_dir)
        );
    }

    #[test]
    fn measure_reports_unreadable_directory() {
        let root = TempDir::new();
        let blocked = root.0.join("blocked");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("secret"), vec![9u8; 4096]).unwrap();
        let mut permissions = fs::metadata(&blocked).unwrap().permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&blocked, permissions).unwrap();

        let result = if fs::read_dir(&blocked).is_ok() {
            None
        } else {
            Some(measure_path_inner(&blocked, None))
        };

        let mut permissions = fs::metadata(&blocked).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&blocked, permissions).unwrap();

        if let Some(result) = result {
            assert!(matches!(result, Err(MeasureError::NotPermitted)));
        }
    }

    #[test]
    fn root_listing_skips_other_mounts() {
        let children = list_children(Path::new("/")).unwrap();
        let paths: Vec<&str> = children.iter().map(|entry| entry.path.as_str()).collect();
        assert!(paths.contains(&"/Users"));
        assert!(!paths.contains(&"/Volumes"));
        assert!(!paths.contains(&"/dev"));
        assert!(!paths.iter().any(|path| path.starts_with("/System/Volumes")));
    }

    #[test]
    fn home_listing_keeps_protected_folders_on_their_rows() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let children = list_children(&home).unwrap();
        assert!(children.len() > 1);
        for name in ["Desktop", "Documents", "Downloads"] {
            let Some(entry) = children.iter().find(|entry| entry.name == name) else {
                continue;
            };
            assert!(entry.is_dir, "{name}");
            if fs::read_dir(&entry.path).is_err() {
                let result = measure_path_inner(Path::new(&entry.path), None);
                assert!(
                    matches!(result, Err(MeasureError::NotPermitted)),
                    "{name} should stay on its row as not permitted, got {result:?}"
                );
            }
        }
    }

    #[test]
    fn sized_event_serializes_for_the_frontend() {
        let event = ScanEvent::Sized {
            path: "/Users".into(),
            bytes: 10,
            error: None,
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["event"], "sized");
        assert_eq!(json["data"]["bytes"], 10);
        assert!(json["data"]["error"].is_null());

        let started = ScanEvent::Started {
            entries: vec![ChildEntry {
                name: "Users".into(),
                path: "/Users".into(),
                is_dir: true,
            }],
        };
        let json = serde_json::to_value(&started).unwrap();
        assert_eq!(json["event"], "started");
        assert_eq!(json["data"]["entries"][0]["isDir"], true);
        assert_eq!(
            serde_json::to_value(ScanEvent::Done).unwrap()["event"],
            "done"
        );
    }
}
