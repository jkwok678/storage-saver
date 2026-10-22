use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicU64 as StdAtomic;

use super::bulk::{BulkEntry, read_dir_bulk};
use super::directory::scan_into;
use super::largest::{LargestScan, largest_walk};
use super::listing::{allocated_bytes, is_excluded, is_real_dir, list_children, read_disk_summary};
use super::snapshot::ranked_folders;
use super::trash::move_into_trash;
use super::tree::MeasureError;
use super::walk::{measure_tree, walk_pool};
use super::*;

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
        Self(path.canonicalize().unwrap())
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

    let measured = measure(&root.0).unwrap();
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
            .any(|entry| entry.name == "sub" && entry.is_dir && entry.bytes.is_none())
    );
    assert!(listed.iter().any(|entry| entry.name == "a.txt"
        && !entry.is_dir
        && entry.bytes == Some(file_bytes(&file))));
    assert!(
        listed
            .iter()
            .any(|entry| entry.name == "usr-link" && !entry.is_dir && entry.bytes == Some(0))
    );
    assert!(listed.iter().all(|entry| entry.modified.is_some()));
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

    let (result, nested) = if fs::read_dir(&blocked).is_ok() {
        (None, None)
    } else {
        (Some(measure(&blocked)), Some(measure(&root.0)))
    };

    let mut permissions = fs::metadata(&blocked).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&blocked, permissions).unwrap();

    if let Some(result) = result {
        assert!(matches!(result, Err(MeasureError::NotPermitted)));
        // Below the walk root, the denial is noted on the folder's own node.
        assert!(matches!(nested, Some(Err(MeasureError::NotPermitted))));
    }
}

/// Sizes from `getattrlistbulk` must match `lstat`, which is what `du` and Finder's
/// "on disk" figure use.
fn assert_bulk_matches_lstat(dir: &Path) {
    let mut by_id = HashMap::new();
    let mut folders = Vec::new();
    read_dir_bulk(dir, |entry| match entry {
        BulkEntry::File {
            bytes,
            links,
            dev,
            file_id,
            ..
        } => {
            by_id.insert(file_id, (bytes, links, dev));
        }
        BulkEntry::Folder(name) => folders.push(name),
        BulkEntry::Other => {}
        BulkEntry::Error(errno) => panic!("entry error {errno} in {dir:?}"),
    })
    .unwrap();

    let mut files = HashSet::new();
    let mut expected_folders = Vec::new();
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let metadata = fs::symlink_metadata(entry.path()).unwrap();
        if is_real_dir(&metadata.file_type()) {
            expected_folders.push(entry.file_name().to_string_lossy().into_owned());
        }
        if !metadata.is_file() {
            continue;
        }
        files.insert(metadata.ino());
        let found = by_id.get(&metadata.ino()).copied();
        let expected = (
            allocated_bytes(&metadata),
            metadata.nlink() as u32,
            metadata.dev() as u32 as u64,
        );
        assert_eq!(found, Some(expected), "{:?}", entry.path());
    }
    assert_eq!(by_id.len(), files.len(), "{dir:?}");
    folders.sort();
    expected_folders.sort();
    assert_eq!(folders, expected_folders, "{dir:?}");
}

#[test]
fn bulk_listing_matches_lstat() {
    let root = TempDir::new();
    let dir = &root.0;
    fs::write(dir.join("plain.bin"), vec![7u8; 300 * 1024]).unwrap();
    fs::write(dir.join("empty"), b"").unwrap();
    fs::hard_link(dir.join("plain.bin"), dir.join("linked.bin")).unwrap();
    symlink("plain.bin", dir.join("link")).unwrap();
    fs::create_dir(dir.join("folder")).unwrap();
    fs::create_dir(dir.join("名前 with spaces")).unwrap();
    let sparse = fs::File::create(dir.join("sparse")).unwrap();
    sparse.set_len(64 * 1024 * 1024).unwrap();
    // Long names push entries past any fixed-size assumption.
    fs::write(dir.join("n".repeat(250)), b"x").unwrap();
    let fifo = CString::new(dir.join("fifo").as_os_str().as_bytes()).unwrap();
    // SAFETY: fifo is a valid C string.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    // Transparently compressed copies report their compressed size to `lstat`.
    let compressed = dir.join("compressed.txt");
    let _ = std::process::Command::new("ditto")
        .args(["--hfsCompression", "/usr/share/dict/words"])
        .arg(&compressed)
        .status();
    let _ = std::process::Command::new("xattr")
        .args(["-w", "com.apple.ResourceFork", &"r".repeat(5000)])
        .arg(dir.join("plain.bin"))
        .status();
    // Enough entries to need several bulk calls.
    let many = dir.join("many");
    fs::create_dir(&many).unwrap();
    for index in 0..3000 {
        fs::write(many.join(format!("file-{index:04}")), b"abc").unwrap();
    }

    assert_bulk_matches_lstat(dir);
    assert_bulk_matches_lstat(&many);
    if compressed.exists() {
        let metadata = fs::symlink_metadata(&compressed).unwrap();
        assert!(
            allocated_bytes(&metadata) < metadata.len(),
            "not compressed"
        );
    }
    assert!(file_bytes(&dir.join("sparse")) < 1024 * 1024);
}

#[test]
fn bulk_listing_reports_unreadable_folder() {
    let err = read_dir_bulk(Path::new("/definitely/not/here"), |_| {}).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    let file = TempDir::new();
    fs::write(file.0.join("f"), b"x").unwrap();
    assert!(read_dir_bulk(&file.0.join("f"), |_| {}).is_err());
}

/// Compares every folder under real system and home paths against `lstat`.
/// Slow, so run by hand: `cargo test --release -- --ignored --nocapture`.
#[test]
#[ignore]
fn bulk_listing_matches_lstat_on_real_folders() {
    let home = PathBuf::from(std::env::var("HOME").unwrap());
    let mut checked = 0;
    for root in [
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Library/Frameworks"),
        home.join("Library/Caches"),
        home.join("projects"),
    ] {
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| is_real_dir(&kind)) {
                    stack.push(entry.path());
                }
            }
            // Files can change between the two listings in live folders. Skip a mismatch only there.
            let check = std::panic::catch_unwind(|| assert_bulk_matches_lstat(&dir));
            if check.is_err() && !dir.starts_with(home.join("Library/Caches")) {
                panic!("mismatch in {dir:?}");
            }
            checked += 1;
        }
    }
    eprintln!("checked {checked} folders");
}

#[test]
fn tree_keeps_every_folder_total() {
    let root = TempDir::new();
    let a = root.0.join("a");
    let b = a.join("b");
    let c = root.0.join("c");
    fs::create_dir_all(&b).unwrap();
    fs::create_dir(&c).unwrap();
    fs::create_dir(root.0.join("empty")).unwrap();
    fs::write(root.0.join("top.bin"), vec![1u8; 16 * 1024]).unwrap();
    fs::write(a.join("x.bin"), vec![1u8; 32 * 1024]).unwrap();
    fs::write(b.join("y.bin"), vec![1u8; 64 * 1024]).unwrap();
    fs::write(c.join("z.bin"), vec![1u8; 128 * 1024]).unwrap();
    let top = file_bytes(&root.0.join("top.bin"));
    let x = file_bytes(&a.join("x.bin"));
    let y = file_bytes(&b.join("y.bin"));
    let z = file_bytes(&c.join("z.bin"));

    let tree = measure_tree(&root.0, None, &Arc::default()).unwrap();
    let size = |path: &Path| tree.size_of(tree.find(path).unwrap()).unwrap();
    assert_eq!(size(&root.0), top + x + y + z);
    assert_eq!(size(&a), x + y);
    assert_eq!(size(&b), y);
    assert_eq!(size(&c), z);
    assert_eq!(size(&root.0.join("empty")), 0);
    assert!(tree.find(&root.0.join("missing")).is_none());
    assert!(tree.find(&root.0.join("top.bin")).is_none());
    assert_eq!(tree.nodes.len(), 5);
}

#[test]
fn opening_a_scanned_folder_reads_stored_totals() {
    let root = TempDir::new();
    let a = root.0.join("a");
    let b = a.join("b");
    fs::create_dir_all(&b).unwrap();
    fs::create_dir(root.0.join("c")).unwrap();
    fs::write(a.join("x.bin"), vec![1u8; 32 * 1024]).unwrap();
    fs::write(b.join("y.bin"), vec![1u8; 64 * 1024]).unwrap();
    let x = file_bytes(&a.join("x.bin"));
    let y = file_bytes(&b.join("y.bin"));
    let stored = Mutex::new(None);

    let (top, walked) = scan(&stored, &root.0, false);
    assert_eq!(walked, vec![path_of(&root.0, "a"), path_of(&root.0, "c")]);
    assert_eq!(bytes_of(&top, "a"), x + y);

    let (inside, walked) = scan(&stored, &a, false);
    assert!(walked.is_empty(), "walked {walked:?}");
    assert_eq!(bytes_of(&inside, "b"), y);
    assert_eq!(bytes_of(&inside, "x.bin"), x);

    // A folder made since the scan is walked on its own and joins the stored tree.
    fs::create_dir(a.join("new")).unwrap();
    fs::write(a.join("new").join("n.bin"), vec![1u8; 16 * 1024]).unwrap();
    let n = file_bytes(&a.join("new").join("n.bin"));
    let (inside, walked) = scan(&stored, &a, false);
    assert_eq!(walked, vec![path_of(&a, "new")]);
    assert_eq!(bytes_of(&inside, "new"), n);
    let (top, walked) = scan(&stored, &root.0, false);
    assert!(walked.is_empty(), "walked {walked:?}");
    assert_eq!(bytes_of(&top, "a"), x + y + n);
}

#[test]
fn refresh_walks_again_and_updates_parents() {
    let root = TempDir::new();
    let a = root.0.join("a");
    let b = a.join("b");
    fs::create_dir_all(&b).unwrap();
    fs::create_dir(root.0.join("c")).unwrap();
    fs::write(b.join("y.bin"), vec![1u8; 64 * 1024]).unwrap();
    let y = file_bytes(&b.join("y.bin"));
    let stored = Mutex::new(None);
    scan(&stored, &root.0, false);

    fs::write(b.join("more.bin"), vec![1u8; 256 * 1024]).unwrap();
    let more = file_bytes(&b.join("more.bin"));
    let (stale, _) = scan(&stored, &root.0, false);
    assert_eq!(bytes_of(&stale, "a"), y);

    let (inside, walked) = scan(&stored, &a, true);
    assert_eq!(walked, vec![path_of(&a, "b")]);
    assert_eq!(bytes_of(&inside, "b"), y + more);
    let (top, walked) = scan(&stored, &root.0, false);
    assert!(walked.is_empty(), "walked {walked:?}");
    assert_eq!(bytes_of(&top, "a"), y + more);

    // The replaced copy of `a` is dropped, not kept beside the new one.
    let guard = lock(&stored);
    let tree = guard.as_ref().unwrap();
    assert_eq!(tree.root, root.0);
    assert_eq!(tree.nodes.len(), 4);
}

#[test]
fn going_up_reuses_the_folder_already_scanned() {
    let root = TempDir::new();
    let a = root.0.join("a");
    fs::create_dir_all(a.join("b")).unwrap();
    fs::create_dir(root.0.join("c")).unwrap();
    fs::write(a.join("b").join("y.bin"), vec![1u8; 64 * 1024]).unwrap();
    let y = file_bytes(&a.join("b").join("y.bin"));
    let stored = Mutex::new(None);

    scan(&stored, &a, false);
    let (top, walked) = scan(&stored, &root.0, false);
    assert_eq!(walked, vec![path_of(&root.0, "c")]);
    assert_eq!(bytes_of(&top, "a"), y);
    assert_eq!(lock(&stored).as_ref().unwrap().root, root.0);
    let (_, walked) = scan(&stored, &a, false);
    assert!(walked.is_empty(), "walked {walked:?}");
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
            let result = measure(Path::new(&entry.path));
            assert!(
                matches!(result, Err(MeasureError::NotPermitted)),
                "{name} should stay on its row as not permitted, got {result:?}"
            );
        }
    }
}

#[test]
fn largest_files_rank_by_size_and_skip_hard_links() {
    let root = TempDir::new();
    let nested = root.0.join("nested");
    fs::create_dir(&nested).unwrap();
    fs::write(root.0.join("small.bin"), vec![1u8; 8 * 1024]).unwrap();
    fs::write(root.0.join("medium.bin"), vec![2u8; 64 * 1024]).unwrap();
    let big = nested.join("big.bin");
    fs::write(&big, vec![3u8; 256 * 1024]).unwrap();
    fs::hard_link(&big, root.0.join("big-link.bin")).unwrap();

    let files = largest_in(&root.0, 10);
    assert_eq!(files.len(), 3, "{files:?}");
    assert_eq!(files[0].bytes, file_bytes(&big));
    assert_eq!(files[1].bytes, file_bytes(&root.0.join("medium.bin")));
    assert_eq!(files[2].bytes, file_bytes(&root.0.join("small.bin")));
    assert!(
        files[0].name == "big.bin" || files[0].name == "big-link.bin",
        "hard links should appear once, got {:?}",
        files[0]
    );

    let top = largest_in(&root.0, 2);
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].bytes, file_bytes(&big));
    assert_eq!(top[1].bytes, file_bytes(&root.0.join("medium.bin")));
}

#[test]
fn largest_event_serializes_for_the_frontend() {
    let event = LargestEvent::Batch {
        files: vec![LargestFile {
            name: "movie.mov".into(),
            path: "/Users/me/movie.mov".into(),
            bytes: 123,
        }],
        scanned: 12,
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(json["event"], "batch");
    assert_eq!(json["data"]["scanned"], 12);
    assert_eq!(json["data"]["files"][0]["name"], "movie.mov");
    assert_eq!(json["data"]["files"][0]["bytes"], 123);

    let done = serde_json::to_value(LargestEvent::Done { scanned: 12 }).unwrap();
    assert_eq!(done["event"], "done");
    assert_eq!(done["data"]["scanned"], 12);
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
            bytes: None,
            modified: Some(1_700_000_000),
            error: None,
        }],
    };
    let json = serde_json::to_value(&started).unwrap();
    assert_eq!(json["event"], "started");
    assert_eq!(json["data"]["entries"][0]["isDir"], true);
    assert_eq!(json["data"]["entries"][0]["modified"], 1_700_000_000u64);
    assert!(json["data"]["entries"][0]["bytes"].is_null());
    assert_eq!(
        serde_json::to_value(ScanEvent::Done).unwrap()["event"],
        "done"
    );
}

#[test]
fn trash_moves_items_with_collision_suffix_and_updates_stored_totals() {
    let root = TempDir::new();
    let trash = TempDir::new();
    let cache = root.0.join("cache");
    let keep_dir = root.0.join("keep");
    fs::create_dir(&cache).unwrap();
    fs::create_dir(&keep_dir).unwrap();
    let junk = cache.join("junk.bin");
    let kept = keep_dir.join("kept.bin");
    fs::write(&junk, vec![4u8; 128 * 1024]).unwrap();
    fs::write(&kept, vec![5u8; 64 * 1024]).unwrap();
    let junk_bytes = file_bytes(&junk);
    let kept_bytes = file_bytes(&kept);

    let stored = Mutex::new(None);
    scan(&stored, &root.0, false);
    let stored_largest = Mutex::new(vec![
        LargestFile {
            name: "junk.bin".into(),
            path: junk.to_string_lossy().into_owned(),
            bytes: junk_bytes,
        },
        LargestFile {
            name: "kept.bin".into(),
            path: kept.to_string_lossy().into_owned(),
            bytes: kept_bytes,
        },
    ]);

    // Pre-populate a colliding name in trash
    fs::create_dir(trash.0.join("cache")).unwrap();

    let outcome =
        trash_path_into(&cache, &trash.0, &stored, &stored_largest, move_plainly).unwrap();
    assert_eq!(outcome.freed_bytes, junk_bytes);
    assert!(outcome.trash_path.ends_with("cache 2"));
    assert!(!cache.exists());

    // Stored tree should reflect that `cache` is gone and root total is now just `kept_bytes`.
    let tree_guard = lock(&stored);
    let tree = tree_guard.as_ref().unwrap();
    assert!(tree.find(&cache).is_none());
    assert_eq!(tree.size_of(0).unwrap(), kept_bytes);
    drop(tree_guard);

    // Stored largest files should no longer contain files from the trashed folder.
    let remaining = lock(&stored_largest);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "kept.bin");
}

/// Tests move into their own folder, never the real Trash through Finder.
fn move_plainly(target: &Path, trash_dir: &Path) -> Result<(PathBuf, bool), String> {
    move_into_trash(target, trash_dir).map(|dest| (dest, false))
}

#[test]
fn protected_paths_cannot_be_trashed() {
    assert!(is_protected_path(Path::new("/")));
    assert!(is_protected_path(Path::new("/System")));
    assert!(is_protected_path(Path::new("/System/Library")));
    assert!(is_protected_path(Path::new("/Applications")));
    assert!(is_protected_path(Path::new("/Users")));
    assert!(is_protected_path(Path::new("/system/library")));
    let home = std::env::var("HOME").unwrap();
    assert!(is_protected_path(&Path::new(&home).join("library")));
    assert!(is_protected_path(
        &Path::new(&home).join("Library/Keychains")
    ));
    assert!(!is_protected_path(&Path::new(&home).join("Library/Caches")));
}

#[test]
fn trash_refuses_paths_that_climb_out() {
    let home = std::env::var("HOME").unwrap();
    let sneaky = Path::new(&home).join("Documents/../Library");
    let trash = TempDir::new();
    let stored = Mutex::new(None);
    let stored_largest = Mutex::new(Vec::new());
    assert!(trash_path_into(&sneaky, &trash.0, &stored, &stored_largest, move_plainly).is_err());
    assert!(Path::new(&home).join("Library").exists());
}

#[test]
fn trashing_the_stored_root_forgets_the_tree() {
    let root = TempDir::new();
    let trash = TempDir::new();
    let folder = root.0.join("folder");
    fs::create_dir_all(folder.join("inner")).unwrap();
    fs::write(folder.join("inner/a.bin"), vec![1u8; 4096]).unwrap();
    let stored = Mutex::new(None);
    scan(&stored, &folder, false);
    assert!(lock(&stored).is_some());

    trash_path_into(
        &folder,
        &trash.0,
        &stored,
        &Mutex::new(Vec::new()),
        move_plainly,
    )
    .unwrap();
    assert!(lock(&stored).is_none());
}

#[test]
fn ranked_folders_keep_the_largest_in_order() {
    let mut tree = Tree::new(PathBuf::from("/data"));
    for (index, own) in [5u64, 50, 20, 40].into_iter().enumerate() {
        let id = tree.add_child(0, format!("f{index}").into());
        tree.nodes[id as usize].own = own;
    }
    tree.roll_up();
    let top = ranked_folders(&tree, 2, 10, |node| node.own);
    let paths: Vec<&str> = top.iter().map(|item| item.path.as_str()).collect();
    assert_eq!(paths, ["/data/f1", "/data/f3"]);
}
