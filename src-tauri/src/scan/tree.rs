//! In-memory folder tree. Files are not nodes; their bytes sit on the folder that holds them.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use super::types::ChildEntry;

const NO_NODE: u32 = u32::MAX;
pub(super) const DENIED: u8 = 1;
const FAILED: u8 = 2;

#[derive(Debug)]
pub(super) enum MeasureError {
    NotPermitted,
    Message(String),
}

/// One folder. Files are not kept, only added into their folder's `own`.
#[derive(Debug)]
pub(super) struct Node {
    pub(super) name: Box<str>,
    /// Files directly in this folder.
    pub(super) own: u64,
    /// `own` plus every folder below. Set by `roll_up`.
    pub(super) total: u64,
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
pub struct Tree {
    pub(super) root: PathBuf,
    pub(super) nodes: Vec<Node>,
    /// First error other than "not permitted" for a folder, by node id. Rare, so kept apart.
    messages: HashMap<u32, String>,
}

/// Errors met while reading one folder, folded into its node afterwards.
#[derive(Default)]
pub(super) struct Trouble {
    pub(super) flags: u8,
    pub(super) message: Option<String>,
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
    pub(super) fn new(root: PathBuf) -> Self {
        Self {
            root,
            nodes: vec![Node::new("".into(), NO_NODE)],
            messages: HashMap::new(),
        }
    }

    pub(super) fn add_child(&mut self, parent: u32, name: Box<str>) -> u32 {
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

    pub(super) fn find(&self, path: &Path) -> Option<u32> {
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
    pub(super) fn find_children(&self, dir: &Path, children: &[ChildEntry]) -> Vec<Option<u32>> {
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

    pub(super) fn note(&mut self, id: u32, trouble: Trouble) {
        self.nodes[id as usize].own_flags |= trouble.flags;
        if let Some(message) = trouble.message {
            self.messages.entry(id).or_insert(message);
        }
    }

    pub(super) fn roll_up(&mut self) {
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

    pub(super) fn size_of(&self, id: u32) -> Result<u64, MeasureError> {
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
    pub(super) fn graft(&mut self, parent: u32, name: Box<str>, src: &Tree, src_id: u32) {
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
    pub(super) fn replace(&mut self, id: u32, fresh: &Tree) {
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

    pub(super) fn path_of(&self, mut id: u32) -> PathBuf {
        let mut parts = Vec::new();
        while id != 0 && id != NO_NODE {
            parts.push(&*self.nodes[id as usize].name);
            id = self.nodes[id as usize].parent;
        }
        let mut full = self.root.clone();
        for name in parts.into_iter().rev() {
            full.push(name);
        }
        full
    }

    pub(super) fn remove_node(&mut self, id: u32) {
        if id == 0 || id as usize >= self.nodes.len() {
            return;
        }
        let Node {
            parent,
            next_sibling,
            ..
        } = self.nodes[id as usize];
        if parent == NO_NODE {
            return;
        }
        if self.nodes[parent as usize].first_child == id {
            self.nodes[parent as usize].first_child = next_sibling;
        } else {
            let mut sibling = self.nodes[parent as usize].first_child;
            while sibling != NO_NODE && self.nodes[sibling as usize].next_sibling != id {
                sibling = self.nodes[sibling as usize].next_sibling;
            }
            if sibling != NO_NODE {
                self.nodes[sibling as usize].next_sibling = next_sibling;
            }
        }
        let mut compact = Tree::new(self.root.clone());
        compact.copy_into(0, self, 0);
        compact.roll_up();
        *self = compact;
    }
}

impl Trouble {
    pub(super) fn note_io(&mut self, err: &std::io::Error) {
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
    pub(super) fn text(self) -> String {
        match self {
            MeasureError::NotPermitted => "Not permitted".to_string(),
            MeasureError::Message(message) => message,
        }
    }
}
