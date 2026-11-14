use std::collections::BTreeMap;
use std::process::Command;

use super::*;
use crate::platform::{
    BulkEntry, is_volume_root, list_children, normalize_input, read_dir_bulk, trash_dir_for,
    volume_slot, volume_summary, volumes,
};

fn system_drive() -> String {
    format!(
        "{}\\",
        std::env::var("SystemDrive")
            .unwrap_or_else(|_| "C:".into())
            .to_uppercase()
    )
}

fn env_path(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap())
}

#[test]
fn volumes_include_system_drive() {
    let drive = system_drive();
    let all = volumes();
    let system = all
        .iter()
        .find(|volume| volume.path.to_string_lossy().eq_ignore_ascii_case(&drive))
        .unwrap_or_else(|| panic!("{drive} is not in the list"));
    assert!(system.total > 0);
    assert!(system.available <= system.total);
    assert!(system.error.is_none());
    assert!(system.label.ends_with(&format!("({})", &drive[..2])));

    let summary = volume_summary(Some(Path::new(&drive))).unwrap();
    assert_eq!(summary.total, system.total);
    assert_eq!(summary.used + summary.available, summary.total);
    let inside = volume_summary(Some(Path::new(&format!("{drive}Windows")))).unwrap();
    assert_eq!(inside.total, summary.total);
    let everything = volume_summary(None).unwrap();
    assert!(everything.total >= summary.total);
    assert!(volume_summary(Some(Path::new("\\\\server\\share"))).is_err());
}

#[test]
fn bulk_matches_read_dir() {
    let root = TempDir::new();
    let long_name = "n".repeat(250);
    let unicode_name = "caf\u{e9}-\u{6587}\u{4ef6}-\u{1f600}.txt";
    for index in 0..3000 {
        fs::write(root.0.join(format!("file-{index:05}.dat")), b"x").unwrap();
    }
    fs::write(root.0.join(unicode_name), b"x").unwrap();
    fs::write(root.0.join(&long_name), b"x").unwrap();
    fs::write(root.0.join("big.bin"), vec![1u8; 64 * 1024]).unwrap();
    fs::create_dir(root.0.join("folder")).unwrap();

    let mut expected = BTreeMap::new();
    for entry in fs::read_dir(&root.0).unwrap() {
        let entry = entry.unwrap();
        expected.insert(
            entry.file_name().to_string_lossy().into_owned(),
            entry.file_type().unwrap().is_dir(),
        );
    }
    let mut seen = BTreeMap::new();
    let mut big = None;
    read_dir_bulk(&root.0, |entry| match entry {
        BulkEntry::File { name, bytes, .. } => {
            if name == "big.bin" {
                big = Some(bytes);
            }
            assert!(seen.insert(name, false).is_none());
        }
        BulkEntry::Folder(name) => assert!(seen.insert(name, true).is_none()),
        BulkEntry::Other | BulkEntry::Error(_) => panic!("unexpected entry"),
    })
    .unwrap();
    assert_eq!(seen, expected);
    assert_eq!(seen.len(), 3004);
    assert_eq!(big, Some(file_bytes(&root.0.join("big.bin"))));

    let listed = list_children(&root.0).unwrap();
    assert_eq!(listed.len(), seen.len());
    assert!(
        listed
            .iter()
            .all(|child| child.is_dir == seen[&child.name] && child.modified.is_some())
    );
    assert_eq!(
        bytes_of(&listed, "big.bin"),
        file_bytes(&root.0.join("big.bin"))
    );
}

#[test]
fn unreadable_folder_is_reported() {
    let missing = TempDir::new().0.join("missing");
    assert!(read_dir_bulk(&missing, |_| {}).is_err());
    let err = list_children(&missing).unwrap_err();
    assert!(!err.is_empty());
}

#[test]
fn hard_link_in_two_folders_counted_once() {
    let root = TempDir::new();
    let a = root.0.join("a");
    let b = root.0.join("b");
    fs::create_dir(&a).unwrap();
    fs::create_dir(&b).unwrap();
    let file = a.join("data.bin");
    fs::write(&file, vec![7u8; 256 * 1024]).unwrap();
    fs::hard_link(&file, b.join("link.bin")).unwrap();
    let bytes = file_bytes(&file);
    assert!(bytes >= 256 * 1024);

    assert_eq!(measure(&root.0).unwrap(), bytes);
    let top = largest_in(&root.0, 10);
    assert_eq!(top.len(), 1, "{top:?}");
    assert_eq!(top[0].bytes, bytes);
}

#[test]
fn junction_not_followed() {
    let root = TempDir::new();
    let target = root.0.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("data.bin"), vec![3u8; 128 * 1024]).unwrap();
    let link = root.0.join("link");
    let status = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(status.status.success(), "{status:?}");

    assert_eq!(
        measure(&root.0).unwrap(),
        file_bytes(&target.join("data.bin"))
    );
    let listed = list_children(&root.0).unwrap();
    let link_row = listed.iter().find(|child| child.name == "link").unwrap();
    assert!(!link_row.is_dir);
    assert_eq!(link_row.bytes, Some(0));
    let mut kinds = Vec::new();
    read_dir_bulk(&root.0, |entry| {
        kinds.push(matches!(entry, BulkEntry::Other));
    })
    .unwrap();
    assert_eq!(kinds.iter().filter(|other| **other).count(), 1);
}

#[test]
fn path_over_260_chars_is_measured() {
    let root = TempDir::new();
    let mut deep = root.0.clone();
    for _ in 0..6 {
        deep.push("d".repeat(50));
    }
    fs::create_dir_all(&deep).unwrap();
    let file = deep.join("deep.bin");
    fs::write(&file, vec![9u8; 64 * 1024]).unwrap();
    assert!(file.to_string_lossy().len() > 260);

    let bytes = file_bytes(&file);
    assert!(bytes >= 64 * 1024);
    assert_eq!(measure(&root.0).unwrap(), bytes);
    assert_eq!(largest_in(&root.0, 5)[0].bytes, bytes);
}

#[test]
fn protected_paths_windows() {
    let profile = env_path("USERPROFILE");
    let local = env_path("LOCALAPPDATA");
    let protected = [
        PathBuf::from("C:\\"),
        PathBuf::from("c:/windows/system32"),
        PathBuf::from("\\\\?\\C:\\Windows"),
        PathBuf::from("c:\\program files"),
        PathBuf::from("c:\\users"),
        profile.join("AppData"),
        profile.join("Downloads"),
        profile.join("Documents"),
        local.clone(),
        PathBuf::from("\\\\server\\share\\folder"),
    ];
    for path in protected {
        assert!(is_protected_path(&path), "{}", path.display());
    }
    assert!(!is_protected_path(&local.join("Temp").join("x")));
    assert!(!is_protected_path(
        &profile.join("Downloads").join("setup.exe")
    ));
    assert!(!is_protected_path(Path::new("C:\\storage-saver-scratch")));
}

#[test]
fn recycle_bin_items_refused() {
    let drive = system_drive();
    let bin = trash_dir_for(Path::new(&format!("{drive}Users\\me\\file.txt"))).unwrap();
    assert_eq!(bin, PathBuf::from(format!("{drive}$Recycle.Bin")));
    assert!(trash_dir_for(Path::new("relative\\file.txt")).is_err());

    let item = bin.join("S-1-5-21-1").join("$RABCDEF.txt");
    let err = trash_path_into(
        &item,
        &bin,
        &Mutex::new(None),
        &Mutex::new(Vec::new()),
        move_plainly,
    )
    .unwrap_err();
    assert!(!err.is_empty());

    // An existing item inside the Recycle Bin folder is refused for that reason.
    let root = TempDir::new();
    let fake_bin = root.0.join("$Recycle.Bin");
    let inside = fake_bin.join("S-1-5-21-1").join("$RABCDEF.txt");
    fs::create_dir_all(inside.parent().unwrap()).unwrap();
    fs::write(&inside, b"x").unwrap();
    let err = trash_path_into(
        &inside,
        &fake_bin,
        &Mutex::new(None),
        &Mutex::new(Vec::new()),
        move_plainly,
    )
    .unwrap_err();
    assert_eq!(err, "Item is already in the Trash");
    assert!(inside.exists());
}

#[test]
fn virtual_root_scan_lists_drives_without_storing() {
    assert_eq!(platform::VIRTUAL_ROOT, Some(""));
    let mut entries = Vec::new();
    let mut done = false;
    scan_virtual_root(|event| {
        match event {
            ScanEvent::Started { entries: listed } => entries = listed,
            ScanEvent::Done => done = true,
            ScanEvent::Sized { .. } => panic!("the top level walks nothing"),
        }
        true
    });
    assert!(done);
    let drive = system_drive();
    let system = entries
        .iter()
        .find(|entry| entry.path.eq_ignore_ascii_case(&drive))
        .unwrap();
    assert!(system.is_dir);
    assert!(system.bytes.is_some());
    assert!(STORED.iter().all(|slot| lock(slot).is_none()));
}

#[test]
fn paths_map_to_volume_slots_and_roots() {
    assert_eq!(volume_slot(Path::new("C:\\a\\b")), 3);
    assert_eq!(volume_slot(Path::new("d:/a")), 4);
    assert_eq!(volume_slot(Path::new("A:\\")), 1);
    assert_eq!(volume_slot(Path::new("")), 0);
    assert!(!std::ptr::eq(
        stored_for(Path::new("C:\\a")),
        stored_for(Path::new("D:\\a"))
    ));
    assert!(std::ptr::eq(
        stored_for(Path::new("C:\\a")),
        stored_for(Path::new("c:\\b\\c"))
    ));

    assert!(is_volume_root(Path::new("C:\\")));
    assert!(is_volume_root(Path::new("d:")));
    assert!(!is_volume_root(Path::new("C:\\Windows")));
}

#[test]
fn input_paths_are_normalised() {
    assert_eq!(normalize_input("c:/Users/me/").unwrap(), "C:\\Users\\me");
    assert_eq!(normalize_input("").unwrap(), "");
    assert!(normalize_input("\\\\server\\share").is_err());
    assert!(normalize_input("C:\\a\\..\\b").is_err());
}

#[test]
fn trash_dir_follows_the_drive_of_the_item() {
    assert_eq!(
        trash_dir_for(Path::new("d:/data/x")).unwrap(),
        PathBuf::from("D:\\$Recycle.Bin")
    );
}
