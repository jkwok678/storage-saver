use jwalk::{DirEntry, Parallelism, WalkDirGeneric};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs::{self, FileType};
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use tauri::ipc::Channel;

/// Latest scan wins. Older walks notice the generation change and stop.
static SCAN_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Folder totals from finished scans. Opening a folder reads them instead of walking it again.
static STORED: Mutex<Option<Tree>> = Mutex::new(None);

const NO_NODE: u32 = u32::MAX;
const DENIED: u8 = 1;
const FAILED: u8 = 2;

/// jwalk client state: the node id of the folder being read, and of each entry.
type NodeIds = (u32, u32);

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

    fn note_walk(&mut self, err: &jwalk::Error) {
        match err.io_error() {
            Some(io_error) => self.note_io(io_error),
            None => self.fail(err.to_string()),
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
    children.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
        let (bytes, error) = if is_dir {
            (None, None)
        } else if file_type.is_file() {
            match fs::symlink_metadata(&path) {
                Ok(metadata) => (Some(allocated_bytes(&metadata)), None),
                Err(err) => {
                    let mut trouble = Trouble::default();
                    trouble.note_io(&err);
                    if trouble.flags & DENIED != 0 {
                        (None, Some(MeasureError::NotPermitted.text()))
                    } else {
                        (Some(0), trouble.message)
                    }
                }
            }
        } else {
            // Links, sockets and the like take no space of their own worth showing.
            (Some(0), None)
        };
        children.push(ChildEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            is_dir,
            bytes,
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
    let mut trouble = Trouble::default();
    let mut walkable = false;
    match fs::symlink_metadata(path) {
        // jwalk records an unreadable root on the root entry. Checking first keeps that simple.
        Ok(metadata) if is_real_dir(&metadata.file_type()) => match fs::read_dir(path) {
            Ok(_) => walkable = true,
            Err(err) => trouble.note_io(&err),
        },
        Ok(metadata) => {
            if metadata.is_file() {
                tree.nodes[0].own = allocated_bytes(&metadata);
            }
        }
        Err(err) => trouble.note_io(&err),
    }
    if !walkable {
        tree.note(0, trouble);
        tree.roll_up();
        return Ok(tree);
    }

    let shared = Arc::new(Mutex::new(tree));
    let walk = WalkDirGeneric::<NodeIds>::new(path)
        .skip_hidden(false)
        .follow_links(false)
        .parallelism(Parallelism::RayonExistingPool {
            pool: walk_pool(),
            // Callers wait on std threads, not this pool, so a busy pool should queue
            // instead of aborting the walk.
            busy_timeout: None,
        })
        .process_read_dir({
            let shared = shared.clone();
            let links = links.clone();
            move |depth, _dir, dir_id, children| {
                record_dir(&shared, &links, generation, depth, *dir_id, children);
            }
        });

    // File sizes are added up on the walk pool in `record_dir`. This loop only drains
    // the walk, which now yields folders alone, and notes folders that could not be read.
    for entry in walk {
        if generation.is_some_and(scan_cancelled) {
            return Err(Cancelled);
        }
        let mut trouble = Trouble::default();
        let id = match &entry {
            Ok(entry) => {
                match entry
                    .read_children
                    .as_ref()
                    .and_then(|children| children.error())
                {
                    Some(err) => trouble.note_walk(err),
                    None => continue,
                }
                entry.client_state
            }
            Err(err) => {
                trouble.note_walk(err);
                0
            }
        };
        lock(&shared).note(id, trouble);
    }

    let mut tree = std::mem::replace(&mut *lock(&shared), Tree::new(PathBuf::new()));
    tree.roll_up();
    Ok(tree)
}

/// Runs on the walk pool once per folder read. Adds the folder's files into its node,
/// then gives each subfolder a node of its own so its read knows where to add.
fn record_dir(
    tree: &Mutex<Tree>,
    links: &Mutex<HashSet<(u64, u64)>>,
    generation: Option<u64>,
    depth: Option<usize>,
    dir_id: u32,
    children: &mut Vec<jwalk::Result<DirEntry<NodeIds>>>,
) {
    if generation.is_some_and(scan_cancelled) {
        children.clear();
        return;
    }
    // The walk root arrives alone, before any folder is read. It is node 0.
    if depth.is_none() {
        for entry in children.iter_mut().flatten() {
            set_node(entry, 0);
        }
        return;
    }

    let mut own = 0u64;
    let mut trouble = Trouble::default();
    children.retain(|child| {
        let entry = match child {
            Ok(entry) => entry,
            Err(err) => {
                trouble.note_walk(err);
                return false;
            }
        };
        if is_real_dir(&entry.file_type) {
            return !is_excluded(&entry.path());
        }
        if entry.file_type.is_file() {
            match entry.metadata() {
                Ok(metadata) => {
                    let repeat = metadata.nlink() > 1
                        && !lock(links).insert((metadata.dev(), metadata.ino()));
                    if !repeat {
                        own = own.saturating_add(allocated_bytes(&metadata));
                    }
                }
                Err(err) => trouble.note_walk(&err),
            }
        }
        // Files are counted. Only folders go on to the walk.
        false
    });

    let names: Vec<Box<str>> = children
        .iter()
        .flatten()
        .map(|entry| entry.file_name.to_string_lossy().into())
        .collect();
    let mut tree = lock(tree);
    let node = &mut tree.nodes[dir_id as usize];
    node.own = node.own.saturating_add(own);
    tree.note(dir_id, trouble);
    for (entry, name) in children.iter_mut().flatten().zip(names) {
        let id = tree.add_child(dir_id, name);
        set_node(entry, id);
    }
}

fn set_node(entry: &mut DirEntry<NodeIds>, id: u32) {
    entry.client_state = id;
    if let Some(read_children) = entry.read_children.as_mut() {
        read_children.client_read_state = Some(id);
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
                error: None,
            }],
        };
        let json = serde_json::to_value(&started).unwrap();
        assert_eq!(json["event"], "started");
        assert_eq!(json["data"]["entries"][0]["isDir"], true);
        assert!(json["data"]["entries"][0]["bytes"].is_null());
        assert_eq!(
            serde_json::to_value(ScanEvent::Done).unwrap()["event"],
            "done"
        );
    }
}
