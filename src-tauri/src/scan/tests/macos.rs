use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::io::ErrorKind;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

use super::*;
use crate::platform::{
    ADVISOR_RULES, BulkEntry, is_excluded, list_children, read_dir_bulk, volume_summary,
};
use crate::scan::listing::is_real_dir;

#[test]
fn disk_summary_matches_boot_volume_capacity() {
    let summary = volume_summary(None).unwrap();
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
    let mut by_name = HashMap::new();
    let mut folders = Vec::new();
    read_dir_bulk(dir, |entry| match entry {
        BulkEntry::File { name, bytes, key } => {
            by_name.insert(name, (bytes, key));
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
        let name = entry.file_name().to_string_lossy().into_owned();
        let found = by_name.get(&name).copied();
        let key = (metadata.nlink() > 1).then_some((metadata.dev() as u32 as u64, metadata.ino()));
        let expected = (allocated_bytes(&entry.path(), &metadata), key);
        assert_eq!(found, Some(expected), "{:?}", entry.path());
        files.insert(name);
    }
    assert_eq!(by_name.len(), files.len(), "{dir:?}");
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
            allocated_bytes(&compressed, &metadata) < metadata.len(),
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
fn advisor_rules_name_macos() {
    assert!(ADVISOR_RULES.starts_with("You are the Storage Saver AI Advisor for macOS."));
}
