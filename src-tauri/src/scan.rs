use serde::Serialize;
use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::ffi::CString;
use std::fs::{self, FileType};
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use tauri::ipc::Channel;

/// Latest scan wins. Older walks notice the generation change and stop.
static SCAN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Latest largest-files scan wins. Kept apart from `SCAN_GENERATION` so browsing and
/// ranking can be stopped independently when the view changes.
static LARGEST_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Folder totals from finished scans. Opening a folder reads them instead of walking it again.
static STORED: Mutex<Option<Tree>> = Mutex::new(None);

const NO_NODE: u32 = u32::MAX;
const DENIED: u8 = 1;
const FAILED: u8 = 2;

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
    /// Known up front for files and stored folders. `None` until a walk measures the folder.
    bytes: Option<u64>,
    /// Last modification time, in seconds since the Unix epoch.
    modified: Option<u64>,
    error: Option<String>,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LargestFile {
    pub name: String,
    pub path: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "event", content = "data")]
pub enum LargestEvent {
    Batch {
        files: Vec<LargestFile>,
        scanned: u64,
    },
    Done {
        scanned: u64,
    },
}

#[derive(Debug)]
enum MeasureError {
    NotPermitted,
    Message(String),
}

#[derive(Debug)]
struct Cancelled;

/// One folder. Files are not kept, only added into their folder's `own`.
#[derive(Debug)]
struct Node {
    name: Box<str>,
    /// Files directly in this folder.
    own: u64,
    /// `own` plus every folder below. Set by `roll_up`.
    total: u64,
    parent: u32,
    first_child: u32,
    next_sibling: u32,
    own_flags: u8,
    /// `own_flags` of this folder and every folder below. Set by `roll_up`.
    flags: u8,
}

/// Every folder under `root`. A parent always has a smaller id than its children,
/// so totals add up in one pass from the end.
#[derive(Debug)]
struct Tree {
    root: PathBuf,
    nodes: Vec<Node>,
    /// First error other than "not permitted" for a folder, by node id. Rare, so kept apart.
    messages: HashMap<u32, String>,
}

/// Errors met while reading one folder, folded into its node afterwards.
#[derive(Default)]
struct Trouble {
    flags: u8,
    message: Option<String>,
}

impl Node {
    fn new(name: Box<str>, parent: u32) -> Self {
        Self {
            name,
            own: 0,
            total: 0,
            parent,
            first_child: NO_NODE,
            next_sibling: NO_NODE,
            own_flags: 0,
            flags: 0,
        }
    }
}

impl Tree {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            nodes: vec![Node::new("".into(), NO_NODE)],
            messages: HashMap::new(),
        }
    }

    fn add_child(&mut self, parent: u32, name: Box<str>) -> u32 {
        let id = self.nodes.len() as u32;
        let mut node = Node::new(name, parent);
        node.next_sibling = self.nodes[parent as usize].first_child;
        self.nodes[parent as usize].first_child = id;
        self.nodes.push(node);
        id
    }

    fn children(&self, id: u32) -> impl Iterator<Item = u32> + '_ {
        let first = self.nodes[id as usize].first_child;
        std::iter::successors((first != NO_NODE).then_some(first), move |&child| {
            let next = self.nodes[child as usize].next_sibling;
            (next != NO_NODE).then_some(next)
        })
    }

    fn find(&self, path: &Path) -> Option<u32> {
        let rest = path.strip_prefix(&self.root).ok()?;
        let mut id = 0;
        for component in rest.components() {
            let Component::Normal(name) = component else {
                return None;
            };
            let name = name.to_string_lossy();
            id = self
                .children(id)
                .find(|&child| *self.nodes[child as usize].name == *name)?;
        }
        Some(id)
    }

    /// The stored node for each listed folder of `dir`, in the same order as `children`.
    fn find_children(&self, dir: &Path, children: &[ChildEntry]) -> Vec<Option<u32>> {
        let Some(dir_id) = self.find(dir) else {
            // `dir` sits above the stored root, so only the root itself can be one of its folders.
            return children
                .iter()
                .map(|child| (child.is_dir && Path::new(&child.path) == self.root).then_some(0))
                .collect();
        };
        let by_name: HashMap<&str, u32> = self
            .children(dir_id)
            .map(|child| (&*self.nodes[child as usize].name, child))
            .collect();
        children
            .iter()
            .map(|child| {
                if child.is_dir {
                    by_name.get(child.name.as_str()).copied()
                } else {
                    None
                }
            })
            .collect()
    }

    fn note(&mut self, id: u32, trouble: Trouble) {
        self.nodes[id as usize].own_flags |= trouble.flags;
        if let Some(message) = trouble.message {
            self.messages.entry(id).or_insert(message);
        }
    }

    fn roll_up(&mut self) {
        for node in &mut self.nodes {
            node.total = node.own;
            node.flags = node.own_flags;
        }
        for id in (1..self.nodes.len()).rev() {
            let Node {
                total,
                flags,
                parent,
                ..
            } = self.nodes[id];
            if parent == NO_NODE {
                continue;
            }
            let parent = &mut self.nodes[parent as usize];
            parent.total = parent.total.saturating_add(total);
            parent.flags |= flags;
        }
    }

    fn size_of(&self, id: u32) -> Result<u64, MeasureError> {
        let node = &self.nodes[id as usize];
        if node.total > 0 {
            return Ok(node.total);
        }
        if node.flags & DENIED != 0 {
            return Err(MeasureError::NotPermitted);
        }
        if node.flags & FAILED != 0 {
            let first = self
                .messages
                .iter()
                .filter(|(failed, _)| self.is_within(**failed, id))
                .min_by_key(|(failed, _)| **failed);
            if let Some((_, message)) = first {
                return Err(MeasureError::Message(message.clone()));
            }
        }
        Ok(0)
    }

    fn is_within(&self, mut id: u32, ancestor: u32) -> bool {
        while id != NO_NODE {
            if id == ancestor {
                return true;
            }
            id = self.nodes[id as usize].parent;
        }
        false
    }

    /// Copy `src_id` and everything below it in as a new folder of `parent`.
    fn graft(&mut self, parent: u32, name: Box<str>, src: &Tree, src_id: u32) {
        let id = self.add_child(parent, name);
        self.copy_into(id, src, src_id);
    }

    /// Copy the contents of `src_id` onto `dst`, which has no folders yet.
    fn copy_into(&mut self, dst: u32, src: &Tree, src_id: u32) {
        let mut stack = vec![(src_id, dst)];
        while let Some((from, to)) = stack.pop() {
            let node = &src.nodes[from as usize];
            self.nodes[to as usize].own = node.own;
            self.nodes[to as usize].own_flags = node.own_flags;
            if let Some(message) = src.messages.get(&from) {
                self.messages.insert(to, message.clone());
            }
            for child in src.children(from) {
                let id = self.add_child(to, src.nodes[child as usize].name.clone());
                stack.push((child, id));
            }
        }
    }

    /// Swap the folder at `id` for `fresh`. Copying from the root drops the old folder's nodes.
    fn replace(&mut self, id: u32, fresh: &Tree) {
        let Node {
            parent,
            next_sibling,
            ref name,
            ..
        } = self.nodes[id as usize];
        let name = name.clone();
        if self.nodes[parent as usize].first_child == id {
            self.nodes[parent as usize].first_child = next_sibling;
        } else {
            let mut sibling = self.nodes[parent as usize].first_child;
            while self.nodes[sibling as usize].next_sibling != id {
                sibling = self.nodes[sibling as usize].next_sibling;
            }
            self.nodes[sibling as usize].next_sibling = next_sibling;
        }
        self.graft(parent, name, fresh, 0);

        let mut compact = Tree::new(self.root.clone());
        compact.copy_into(0, self, 0);
        compact.roll_up();
        *self = compact;
    }
}

impl Trouble {
    fn note_io(&mut self, err: &std::io::Error) {
        match err.kind() {
            ErrorKind::PermissionDenied => self.flags |= DENIED,
            ErrorKind::NotFound => {}
            _ => self.fail(err.to_string()),
        }
    }

    fn fail(&mut self, message: String) {
        self.flags |= FAILED;
        self.message.get_or_insert(message);
    }
}

impl MeasureError {
    fn text(self) -> String {
        match self {
            MeasureError::NotPermitted => "Not permitted".to_string(),
            MeasureError::Message(message) => message,
        }
    }
}

#[tauri::command]
pub fn disk_summary() -> Result<DiskSummary, String> {
    read_disk_summary()
}

#[tauri::command]
pub async fn scan_directory(
    path: String,
    refresh: bool,
    on_event: Channel<ScanEvent>,
) -> Result<(), String> {
    // The walk is long. Run it off the UI thread so folder names and sizes can paint as they arrive.
    match tauri::async_runtime::spawn_blocking(move || run_scan(path, refresh, on_event)).await {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub fn stop_scan() {
    SCAN_GENERATION.fetch_add(1, Ordering::AcqRel);
}

#[tauri::command]
pub async fn scan_largest_files(
    path: String,
    limit: usize,
    on_event: Channel<LargestEvent>,
) -> Result<(), String> {
    match tauri::async_runtime::spawn_blocking(move || run_largest_scan(path, limit, on_event))
        .await
    {
        Ok(result) => result,
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub fn stop_largest_scan() {
    LARGEST_GENERATION.fetch_add(1, Ordering::AcqRel);
}

fn run_scan(path: String, refresh: bool, on_event: Channel<ScanEvent>) -> Result<(), String> {
    let generation = SCAN_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let root = PathBuf::from(&path);
    if !root.is_absolute() {
        return Err("path must be absolute".into());
    }
    scan_into(&STORED, &root, refresh, Some(generation), |event| {
        on_event.send(event).is_ok()
    })
}

/// List `root`, fill in what the stored tree already knows, and walk only the folders it
/// does not have (or every folder, on `refresh`). `send` returns false once nobody is listening.
fn scan_into(
    stored: &Mutex<Option<Tree>>,
    root: &Path,
    refresh: bool,
    generation: Option<u64>,
    mut send: impl FnMut(ScanEvent) -> bool,
) -> Result<(), String> {
    let cancelled = || generation.is_some_and(scan_cancelled);
    let stop = || {
        if let Some(generation) = generation {
            cancel_if_current(generation);
        }
    };

    let mut children = list_children(root)?;
    children.sort_by_cached_key(|entry| entry.name.to_lowercase());
    let pending = fill_stored_sizes(stored, root, &mut children, refresh);

    if cancelled() {
        return Ok(());
    }
    if !send(ScanEvent::Started {
        entries: children.clone(),
    }) {
        stop();
        return Ok(());
    }

    let walk_needed = refresh || !pending.is_empty();
    let walked = if pending.is_empty() {
        HashMap::new()
    } else {
        match walk_folders(pending, generation, &mut send) {
            Some(walked) => walked,
            None => return Ok(()),
        }
    };

    if cancelled() {
        return Ok(());
    }
    if walk_needed {
        store_scan(stored, generation, root, &children, walked);
    }
    if !send(ScanEvent::Done) {
        stop();
    }
    Ok(())
}

/// Give each stored folder its size. Returns the folders that still need a walk.
fn fill_stored_sizes(
    stored: &Mutex<Option<Tree>>,
    root: &Path,
    children: &mut [ChildEntry],
    refresh: bool,
) -> Vec<String> {
    let stored = lock(stored);
    let tree = stored.as_ref().filter(|_| !refresh);
    let ids = tree
        .map(|tree| tree.find_children(root, children))
        .unwrap_or_default();

    let mut pending = Vec::new();
    for (index, child) in children.iter_mut().enumerate() {
        if !child.is_dir {
            continue;
        }
        match (tree, ids.get(index).copied().flatten()) {
            (Some(tree), Some(id)) => {
                let (bytes, error) = size_fields(tree.size_of(id));
                child.bytes = Some(bytes);
                child.error = error;
            }
            _ => pending.push(child.path.clone()),
        }
    }
    pending
}

/// Walk each folder on its own worker and report each total as it lands.
/// Returns `None` when the scan stopped early.
fn walk_folders(
    folders: Vec<String>,
    generation: Option<u64>,
    send: &mut impl FnMut(ScanEvent) -> bool,
) -> Option<HashMap<String, Tree>> {
    // Shared across the folders so a file linked from two of them counts once.
    let links = Arc::new(Mutex::new(HashSet::new()));
    let workers = worker_count(folders.len());
    let queue = Mutex::new(folders);
    let mut walked = HashMap::new();
    let mut completed = true;

    std::thread::scope(|scope| {
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..workers {
            let tx = tx.clone();
            let queue = &queue;
            let links = &links;
            scope.spawn(move || {
                loop {
                    if generation.is_some_and(scan_cancelled) {
                        break;
                    }
                    let Some(path) = lock(queue).pop() else { break };
                    let measured = measure_tree(Path::new(&path), generation, links);
                    if tx.send((path, measured)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);

        while let Ok((path, measured)) = rx.recv() {
            if generation.is_some_and(scan_cancelled) {
                completed = false;
                break;
            }
            let Ok(tree) = measured else {
                completed = false;
                break;
            };
            let (bytes, error) = size_fields(tree.size_of(0));
            walked.insert(path.clone(), tree);
            if !send(ScanEvent::Sized { path, bytes, error }) {
                if let Some(generation) = generation {
                    cancel_if_current(generation);
                }
                completed = false;
                break;
            }
        }
    });

    completed.then_some(walked)
}

fn run_largest_scan(
    path: String,
    limit: usize,
    on_event: Channel<LargestEvent>,
) -> Result<(), String> {
    let generation = LARGEST_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let root = PathBuf::from(&path);
    if !root.is_absolute() {
        return Err("path must be absolute".into());
    }
    let metadata = fs::symlink_metadata(&root).map_err(|err| err.to_string())?;
    if !is_real_dir(&metadata.file_type()) {
        return Err("path must be a folder".into());
    }
    let scan = LargestScan::new(limit.clamp(1, 1_000));

    std::thread::scope(|scope| {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let events = &on_event;
        let scan_ref = &scan;
        scope.spawn(move || {
            let mut last_scanned = u64::MAX;
            loop {
                match done_rx.recv_timeout(Duration::from_millis(300)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if largest_cancelled(generation) {
                            break;
                        }
                        let scanned = scan_ref.scanned.load(Ordering::Relaxed);
                        if scanned == last_scanned {
                            continue;
                        }
                        last_scanned = scanned;
                        let files = scan_ref.snapshot();
                        if events.send(LargestEvent::Batch { files, scanned }).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        walk_pool().scope(|walk_scope| {
            largest_walk(walk_scope, &scan, root.clone(), Some(generation));
        });
        drop(done_tx);
    });

    if largest_cancelled(generation) {
        return Ok(());
    }
    let scanned = scan.scanned.load(Ordering::Relaxed);
    let _ = on_event.send(LargestEvent::Batch {
        files: scan.snapshot(),
        scanned,
    });
    let _ = on_event.send(LargestEvent::Done { scanned });
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RankedFile {
    bytes: u64,
    path: String,
    name: String,
}

impl Ord for RankedFile {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.bytes
            .cmp(&other.bytes)
            .then_with(|| self.path.cmp(&other.path))
    }
}

impl PartialOrd for RankedFile {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// The largest files seen so far, kept as a min-heap so only `limit` are held at once.
struct LargestScan {
    limit: usize,
    files: Mutex<BinaryHeap<Reverse<RankedFile>>>,
    /// Hard links count once, as they do in folder sizes.
    links: Mutex<HashSet<(u64, u64)>>,
    scanned: AtomicU64,
}

impl LargestScan {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            files: Mutex::new(BinaryHeap::with_capacity(limit)),
            links: Mutex::new(HashSet::new()),
            scanned: AtomicU64::new(0),
        }
    }

    fn add(&self, dir: &Path, files: Vec<(String, u64)>) {
        if files.is_empty() {
            return;
        }
        let mut heap = lock(&self.files);
        for (name, bytes) in files {
            if heap.len() >= self.limit {
                let Some(smallest) = heap.peek().map(|file| file.0.bytes) else {
                    continue;
                };
                if bytes <= smallest {
                    continue;
                }
            }
            heap.push(Reverse(RankedFile {
                bytes,
                path: dir.join(&name).to_string_lossy().into_owned(),
                name,
            }));
            if heap.len() > self.limit {
                heap.pop();
            }
        }
    }

    fn snapshot(&self) -> Vec<LargestFile> {
        let heap = lock(&self.files);
        let mut files: Vec<&RankedFile> = heap.iter().map(|file| &file.0).collect();
        files.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
        files
            .into_iter()
            .map(|file| LargestFile {
                name: file.name.clone(),
                path: file.path.clone(),
                bytes: file.bytes,
            })
            .collect()
    }
}

/// Walk `dir` and keep its files in `scan`, queueing subfolders as tasks of their own.
fn largest_walk<'s>(
    scope: &rayon::Scope<'s>,
    scan: &'s LargestScan,
    dir: PathBuf,
    generation: Option<u64>,
) {
    if generation.is_some_and(largest_cancelled) {
        return;
    }

    let mut files = Vec::new();
    let mut folders = Vec::new();
    // An unreadable folder simply contributes no files to the ranking.
    let _ = read_dir_bulk(&dir, |entry| match entry {
        BulkEntry::File {
            name,
            bytes,
            links,
            dev,
            file_id,
        } => {
            let repeat = links > 1 && !lock(&scan.links).insert((dev, file_id));
            if !repeat {
                files.push((name, bytes));
            }
        }
        BulkEntry::Folder(name) => folders.push(name),
        BulkEntry::Other | BulkEntry::Error(_) => {}
    });
    folders.retain(|name| !is_excluded(&dir.join(name)));

    // Count only after ranking, so the reporter never snapshots a count ahead of the heap.
    let count = files.len() as u64;
    scan.add(&dir, files);
    scan.scanned.fetch_add(count, Ordering::Relaxed);
    for name in folders {
        let path = dir.join(name);
        scope.spawn(move |scope| largest_walk(scope, scan, path, generation));
    }
}

fn largest_cancelled(generation: u64) -> bool {
    LARGEST_GENERATION.load(Ordering::Acquire) != generation
}

/// Build the tree for `root` from fresh walks and stored folders, and keep it.
fn store_scan(
    stored: &Mutex<Option<Tree>>,
    generation: Option<u64>,
    root: &Path,
    children: &[ChildEntry],
    mut walked: HashMap<String, Tree>,
) {
    let mut stored = lock(stored);
    // Checked under the lock, so a scan that was replaced cannot overwrite the newer one's tree.
    if generation.is_some_and(scan_cancelled) {
        return;
    }

    let mut tree = Tree::new(root.to_path_buf());
    let stored_ids = stored
        .as_ref()
        .map(|old| old.find_children(root, children))
        .unwrap_or_default();
    for (index, child) in children.iter().enumerate() {
        if !child.is_dir {
            tree.nodes[0].own = tree.nodes[0].own.saturating_add(child.bytes.unwrap_or(0));
            continue;
        }
        let name: Box<str> = child.name.as_str().into();
        if let Some(fresh) = walked.remove(&child.path) {
            tree.graft(0, name, &fresh, 0);
        } else if let (Some(old), Some(id)) =
            (stored.as_ref(), stored_ids.get(index).copied().flatten())
        {
            tree.graft(0, name, old, id);
        } else {
            // A folder with no total would leave the parent short. Keep the old tree instead.
            return;
        }
    }
    tree.roll_up();
    keep(&mut stored, tree);
}

/// A folder under the stored root replaces its old copy. Anything else becomes the new root.
fn keep(stored: &mut Option<Tree>, fresh: Tree) {
    if let Some(tree) = stored.as_mut()
        && fresh.root != tree.root
        && fresh.root.starts_with(&tree.root)
    {
        if let Some(id) = tree.find(&fresh.root) {
            tree.replace(id, &fresh);
        } else if let Some(parent) = fresh.root.parent().and_then(|parent| tree.find(parent)) {
            let name = fresh
                .root
                .file_name()
                .map(|name| name.to_string_lossy().into())
                .unwrap_or_default();
            tree.graft(parent, name, &fresh, 0);
            tree.roll_up();
        }
        return;
    }
    *stored = Some(fresh);
}

fn size_fields(size: Result<u64, MeasureError>) -> (u64, Option<String>) {
    match size {
        Ok(bytes) => (bytes, None),
        Err(err) => (0, Some(err.text())),
    }
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
        let is_dir = is_real_dir(&file_type);
        let metadata = fs::symlink_metadata(&path);
        let modified = metadata.as_ref().ok().and_then(modified_secs);
        let (bytes, error) = if is_dir {
            (None, None)
        } else if let Ok(metadata) = &metadata {
            // Links, sockets and the like take no space of their own worth showing.
            let bytes = if file_type.is_file() {
                allocated_bytes(metadata)
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

/// Walk `path` and record the total of every folder under it.
fn measure_tree(
    path: &Path,
    generation: Option<u64>,
    links: &Arc<Mutex<HashSet<(u64, u64)>>>,
) -> Result<Tree, Cancelled> {
    if generation.is_some_and(scan_cancelled) {
        return Err(Cancelled);
    }

    let mut tree = Tree::new(path.to_path_buf());
    match fs::symlink_metadata(path) {
        Ok(metadata) if is_real_dir(&metadata.file_type()) => {}
        Ok(metadata) => {
            if metadata.is_file() {
                tree.nodes[0].own = allocated_bytes(&metadata);
            }
            tree.roll_up();
            return Ok(tree);
        }
        Err(err) => {
            let mut trouble = Trouble::default();
            trouble.note_io(&err);
            tree.note(0, trouble);
            tree.roll_up();
            return Ok(tree);
        }
    }

    let walk = Walk {
        tree: Mutex::new(tree),
        links: links.clone(),
        generation,
    };
    walk_pool().scope(|scope| visit(scope, &walk, path.to_path_buf(), 0));
    if walk.cancelled() {
        return Err(Cancelled);
    }
    let mut tree = walk
        .tree
        .into_inner()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tree.roll_up();
    Ok(tree)
}

/// What every folder read in one walk shares.
struct Walk {
    tree: Mutex<Tree>,
    links: Arc<Mutex<HashSet<(u64, u64)>>>,
    generation: Option<u64>,
}

impl Walk {
    fn cancelled(&self) -> bool {
        self.generation.is_some_and(scan_cancelled)
    }
}

/// Runs on the walk pool once per folder. Adds the folder's files into its node, gives
/// each subfolder a node, and queues the subfolders as tasks of their own.
fn visit<'s>(scope: &rayon::Scope<'s>, walk: &'s Walk, dir: PathBuf, id: u32) {
    if walk.cancelled() {
        return;
    }

    let mut own = 0u64;
    let mut trouble = Trouble::default();
    let mut folders = Vec::new();
    let read = read_dir_bulk(&dir, |entry| match entry {
        BulkEntry::File {
            bytes,
            links,
            dev,
            file_id,
            ..
        } => {
            let repeat = links > 1 && !lock(&walk.links).insert((dev, file_id));
            if !repeat {
                own = own.saturating_add(bytes);
            }
        }
        BulkEntry::Folder(name) => folders.push(name),
        BulkEntry::Other => {}
        BulkEntry::Error(errno) => trouble.note_io(&io::Error::from_raw_os_error(errno)),
    });
    if let Err(err) = read {
        trouble.note_io(&err);
    }
    folders.retain(|name| !is_excluded(&dir.join(name)));

    let ids: Vec<u32> = {
        let mut tree = lock(&walk.tree);
        let node = &mut tree.nodes[id as usize];
        node.own = node.own.saturating_add(own);
        tree.note(id, trouble);
        folders
            .iter()
            .map(|name| tree.add_child(id, name.as_str().into()))
            .collect()
    };
    for (name, child) in folders.into_iter().zip(ids) {
        let path = dir.join(name);
        scope.spawn(move |scope| visit(scope, walk, path, child));
    }
}

/// One entry from `getattrlistbulk`.
enum BulkEntry {
    File {
        name: String,
        bytes: u64,
        links: u32,
        dev: u64,
        file_id: u64,
    },
    Folder(String),
    /// Links, sockets and the like. They take no space of their own worth counting.
    Other,
    Error(i32),
}

// Not in the libc crate. Values from <sys/attr.h> and <sys/vnode.h>.
const ATTR_CMN_ERROR: u32 = 0x2000_0000;
const VREG: u32 = 1;
const VDIR: u32 = 2;

const BULK_COMMON: u32 = libc::ATTR_CMN_RETURNED_ATTRS
    | libc::ATTR_CMN_NAME
    | libc::ATTR_CMN_DEVID
    | libc::ATTR_CMN_OBJTYPE
    | libc::ATTR_CMN_FILEID
    | ATTR_CMN_ERROR;
const BULK_FILE: u32 = libc::ATTR_FILE_LINKCOUNT | libc::ATTR_FILE_ALLOCSIZE;

thread_local! {
    /// Reused across reads. `u64` keeps it 8-byte aligned.
    static BULK_BUFFER: RefCell<Vec<u64>> = RefCell::new(vec![0; 64 * 1024 / 8]);
}

/// List `dir` with `getattrlistbulk`. One call returns the type, allocated size, link count
/// and file id of a whole batch of entries, so files cost no `lstat` of their own.
fn read_dir_bulk(dir: &Path, mut each: impl FnMut(BulkEntry)) -> io::Result<()> {
    let path = CString::new(dir.as_os_str().as_bytes())
        .map_err(|err| io::Error::new(ErrorKind::InvalidInput, err))?;
    // SAFETY: path is a valid C string.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fd was just opened and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };

    let mut request = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: BULK_COMMON,
        volattr: 0,
        dirattr: 0,
        fileattr: BULK_FILE,
        forkattr: 0,
    };
    BULK_BUFFER.with_borrow_mut(|buffer| {
        let size = buffer.len() * 8;
        loop {
            // SAFETY: request and buffer outlive the call, and `size` is the buffer's length in bytes.
            let count = unsafe {
                libc::getattrlistbulk(
                    fd.as_raw_fd(),
                    (&raw mut request).cast(),
                    buffer.as_mut_ptr().cast(),
                    size,
                    0,
                )
            };
            if count < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            if count == 0 {
                return Ok(());
            }
            // SAFETY: the buffer is `size` initialized bytes, and u8 has no alignment needs.
            let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), size) };
            let mut offset = 0;
            for _ in 0..count {
                let length = read_u32(bytes, offset).unwrap_or(0) as usize;
                let Some(entry) = bytes.get(offset..offset + length).filter(|_| length > 0) else {
                    return Err(io::Error::new(
                        ErrorKind::InvalidData,
                        "getattrlistbulk returned a malformed entry",
                    ));
                };
                if let Some(entry) = parse_bulk_entry(entry) {
                    each(entry);
                }
                offset += length;
            }
        }
    })
}

/// Attributes follow the entry's length and returned-attribute set, in bit order,
/// each 4-byte aligned. Ones the file system did not return are left out.
fn parse_bulk_entry(entry: &[u8]) -> Option<BulkEntry> {
    let mut at = 4;
    let take_u32 = |at: &mut usize| {
        let value = read_u32(entry, *at);
        *at += 4;
        value
    };
    let common = take_u32(&mut at)?;
    let _volume = take_u32(&mut at)?;
    let _dir = take_u32(&mut at)?;
    let file = take_u32(&mut at)?;
    let _fork = take_u32(&mut at)?;

    let mut name: &[u8] = &[];
    if common & libc::ATTR_CMN_NAME != 0 {
        // attrreference_t: offset from the reference itself, then length with the NUL.
        let reference = at;
        let offset = read_u32(entry, at)? as i32;
        let length = read_u32(entry, at + 4)? as usize;
        at += 8;
        let start = reference.checked_add_signed(offset as isize)?;
        name = entry.get(start..start + length)?;
        name = name.strip_suffix(&[0]).unwrap_or(name);
    }
    let dev = if common & libc::ATTR_CMN_DEVID != 0 {
        take_u32(&mut at)?
    } else {
        0
    };
    let kind = if common & libc::ATTR_CMN_OBJTYPE != 0 {
        take_u32(&mut at)?
    } else {
        0
    };
    let file_id = if common & libc::ATTR_CMN_FILEID != 0 {
        let value = read_u64(entry, at)?;
        at += 8;
        value
    } else {
        0
    };
    if common & ATTR_CMN_ERROR != 0 {
        let errno = take_u32(&mut at)?;
        if errno != 0 {
            return Some(BulkEntry::Error(errno as i32));
        }
    }

    Some(match kind {
        VDIR => BulkEntry::Folder(String::from_utf8_lossy(name).into_owned()),
        VREG => {
            let links = if file & libc::ATTR_FILE_LINKCOUNT != 0 {
                take_u32(&mut at)?
            } else {
                1
            };
            let bytes = if file & libc::ATTR_FILE_ALLOCSIZE != 0 {
                read_u64(entry, at)?
            } else {
                0
            };
            BulkEntry::File {
                name: String::from_utf8_lossy(name).into_owned(),
                bytes,
                links,
                dev: u64::from(dev),
                file_id,
            }
        }
        _ => BulkEntry::Other,
    })
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_ne_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn read_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_ne_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

/// Allocated space, in bytes. `st_blocks` is reported in 512-byte units.
fn allocated_bytes(metadata: &fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

fn modified_secs(metadata: &fs::Metadata) -> Option<u64> {
    u64::try_from(metadata.mtime()).ok()
}

fn walk_pool() -> &'static rayon::ThreadPool {
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(4);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|index| format!("storage-scan-{index}"))
            .build()
            .expect("storage scan thread pool")
    })
}

fn worker_count(jobs: usize) -> usize {
    let cpus = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4);
    jobs.max(1).min(cpus)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
    fn scan(
        stored: &Mutex<Option<Tree>>,
        dir: &Path,
        refresh: bool,
    ) -> (Vec<ChildEntry>, Vec<String>) {
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
}
