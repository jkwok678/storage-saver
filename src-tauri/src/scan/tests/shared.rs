use super::*;

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
    let data = std::env::temp_dir().join("data");
    let mut tree = Tree::new(data.clone());
    for (index, own) in [5u64, 50, 20, 40].into_iter().enumerate() {
        let id = tree.add_child(0, format!("f{index}").into());
        tree.nodes[id as usize].own = own;
    }
    tree.roll_up();
    let top = ranked_folders(&tree, 2, 10, |node| node.own);
    let paths: Vec<&str> = top.iter().map(|item| item.path.as_str()).collect();
    let expected = [data.join("f1"), data.join("f3")];
    let expected: Vec<String> = expected
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(paths, expected);
}

#[test]
fn largest_pruning_keeps_sibling_with_same_prefix() {
    let root = TempDir::new();
    let trash = TempDir::new();
    let cache = root.0.join("cache");
    let sibling = root.0.join("cache2");
    fs::create_dir(&cache).unwrap();
    fs::create_dir(&sibling).unwrap();
    let junk = cache.join("junk.bin");
    let kept = sibling.join("kept.bin");
    fs::write(&junk, vec![4u8; 4096]).unwrap();
    fs::write(&kept, vec![5u8; 4096]).unwrap();
    let stored_largest = Mutex::new(vec![
        LargestFile {
            name: "junk.bin".into(),
            path: junk.to_string_lossy().into_owned(),
            bytes: 4096,
        },
        LargestFile {
            name: "kept.bin".into(),
            path: kept.to_string_lossy().into_owned(),
            bytes: 4096,
        },
    ]);

    trash_path_into(
        &cache,
        &trash.0,
        &Mutex::new(None),
        &stored_largest,
        move_plainly,
    )
    .unwrap();

    let remaining = lock(&stored_largest);
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].name, "kept.bin");
}

#[test]
fn stored_slots_are_per_volume() {
    let root = TempDir::new();
    assert!(std::ptr::eq(
        stored_for(&root.0),
        stored_for(&root.0.join("inside"))
    ));
    #[cfg(target_os = "macos")]
    assert!(std::ptr::eq(
        stored_for(Path::new("/")),
        stored_for(Path::new("/tmp"))
    ));
}

#[test]
fn volume_listing_serializes_for_the_frontend() {
    let volume = Volume {
        path: PathBuf::from("/disk"),
        label: "Data Disk".into(),
        total: 1000,
        available: 400,
        error: None,
    };
    let json = serde_json::to_value(volume_entry(&volume)).unwrap();
    assert_eq!(json["name"], "Data Disk");
    assert_eq!(json["path"], "/disk");
    assert_eq!(json["isDir"], true);
    assert_eq!(json["bytes"], 600);
    assert!(json["modified"].is_null());
    assert!(json["error"].is_null());

    let broken = Volume {
        error: Some("Not ready".into()),
        total: 0,
        available: 0,
        ..volume
    };
    let json = serde_json::to_value(volume_entry(&broken)).unwrap();
    assert_eq!(json["bytes"], 0);
    assert_eq!(json["error"], "Not ready");
}

#[test]
fn listing_the_top_level_stores_nothing() {
    let mut events = Vec::new();
    scan_virtual_root(|event| {
        events.push(event);
        true
    });
    assert!(matches!(
        events.as_slice(),
        [ScanEvent::Started { .. }, ScanEvent::Done]
    ));
    assert!(STORED.iter().all(|slot| lock(slot).is_none()));
}
