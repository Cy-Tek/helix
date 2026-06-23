use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use super::git::GitBadge;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTreeNodeKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTreeEntry {
    pub path: PathBuf,
    pub kind: FileTreeNodeKind,
    pub depth: usize,
    pub git_badge: Option<GitBadge>,
}

impl FileTreeEntry {
    pub fn new(path: PathBuf, kind: FileTreeNodeKind, depth: usize) -> Self {
        Self {
            path,
            kind,
            depth,
            git_badge: None,
        }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self.kind, FileTreeNodeKind::Directory)
    }

    pub fn with_git_badge(mut self, badge: Option<GitBadge>) -> Self {
        self.git_badge = badge;
        self
    }
}

#[derive(Debug, Clone)]
pub struct FileTreeModel {
    root: PathBuf,
    entries: Vec<FileTreeEntry>,
    expanded: BTreeSet<PathBuf>,
    loaded_children: BTreeSet<PathBuf>,
    marked: BTreeSet<PathBuf>,
    selected: usize,
    show_hidden: bool,
    generation: u64,
}

impl FileTreeModel {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            entries: Vec::new(),
            expanded: BTreeSet::new(),
            loaded_children: BTreeSet::new(),
            marked: BTreeSet::new(),
            selected: 0,
            show_hidden: false,
            generation: 0,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn replace_entries(&mut self, entries: Vec<FileTreeEntry>) {
        self.entries = entries;
        self.expanded.clear();
        self.loaded_children.clear();
        self.loaded_children.insert(self.root.clone());
        self.selected = self
            .selected
            .min(self.visible_entries().len().saturating_sub(1));
        self.generation += 1;
    }

    pub fn replace_children(&mut self, parent: &Path, children: Vec<FileTreeEntry>) {
        let Some(parent_index) = self.entries.iter().position(|entry| entry.path == parent) else {
            return;
        };
        let selected_path = self.selected_path().map(Path::to_path_buf);
        let parent_depth = self.entries[parent_index].depth;
        let start = parent_index + 1;
        let end = self.entries[start..]
            .iter()
            .position(|entry| entry.depth <= parent_depth)
            .map(|offset| start + offset)
            .unwrap_or(self.entries.len());

        for entry in &self.entries[start..end] {
            self.loaded_children.remove(&entry.path);
            self.expanded.remove(&entry.path);
        }

        self.entries.splice(start..end, children);
        self.loaded_children.insert(parent.to_path_buf());

        self.selected = selected_path
            .and_then(|path| {
                self.visible_entries()
                    .iter()
                    .position(|entry| entry.path == path)
            })
            .unwrap_or_else(|| {
                self.visible_entries()
                    .iter()
                    .position(|entry| entry.path == parent)
                    .unwrap_or(0)
            });
        self.generation += 1;
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn visible_entries(&self) -> Vec<&FileTreeEntry> {
        self.entries
            .iter()
            .filter(|entry| self.ancestors_are_expanded(entry))
            .collect()
    }

    pub fn visible_paths(&self) -> Vec<PathBuf> {
        self.visible_entries()
            .into_iter()
            .map(|entry| entry.path.clone())
            .collect()
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.visible_entries()
            .get(self.selected)
            .map(|entry| entry.path.as_path())
    }

    pub fn selected_entry(&self) -> Option<&FileTreeEntry> {
        self.visible_entries().get(self.selected).copied()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn select_next(&mut self) {
        let max = self.visible_entries().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(max);
    }

    pub fn select_previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn toggle_mark_selected(&mut self) {
        let Some(path) = self.selected_path().map(Path::to_path_buf) else {
            return;
        };
        if !self.marked.insert(path.clone()) {
            self.marked.remove(&path);
        }
    }

    pub fn marked_paths(&self) -> Vec<PathBuf> {
        self.marked.iter().cloned().collect()
    }

    pub fn operation_targets(&self) -> Vec<PathBuf> {
        let marked = self.marked_paths();
        if !marked.is_empty() {
            return marked;
        }
        self.selected_path()
            .map(Path::to_path_buf)
            .into_iter()
            .collect()
    }

    pub fn create_base_directory(&self) -> PathBuf {
        match self.selected_entry() {
            Some(entry) if entry.is_dir() => entry.path.clone(),
            Some(entry) => entry
                .path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone()),
            None => self.root.clone(),
        }
    }

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    pub fn toggle_expanded(&mut self, path: &Path) {
        if !self.expanded.insert(path.to_path_buf()) {
            self.expanded.remove(path);
        }
    }

    pub fn expand(&mut self, path: &Path) {
        self.expanded.insert(path.to_path_buf());
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
    }

    pub fn children_loaded(&self, path: &Path) -> bool {
        self.loaded_children.contains(path)
    }

    pub fn reveal_path(&mut self, path: &Path) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.path == path) else {
            return false;
        };

        let depth = self.entries[index].depth;
        for ancestor_depth in 0..depth {
            if let Some(ancestor) = self.entries[..index]
                .iter()
                .rev()
                .find(|entry| entry.depth == ancestor_depth && entry.is_dir())
            {
                self.expanded.insert(ancestor.path.clone());
            }
        }

        if let Some(visible_index) = self
            .visible_entries()
            .iter()
            .position(|entry| entry.path == path)
        {
            self.selected = visible_index;
            true
        } else {
            false
        }
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.generation += 1;
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    fn ancestors_are_expanded(&self, entry: &FileTreeEntry) -> bool {
        if entry.depth == 0 {
            return true;
        }

        let Some(index) = self
            .entries
            .iter()
            .position(|candidate| candidate.path == entry.path)
        else {
            return false;
        };

        for depth in 0..entry.depth {
            let Some(ancestor) = self.entries[..index]
                .iter()
                .rev()
                .find(|candidate| candidate.depth == depth && candidate.is_dir())
            else {
                return false;
            };
            if !self.expanded.contains(&ancestor.path) {
                return false;
            }
        }
        true
    }
}
