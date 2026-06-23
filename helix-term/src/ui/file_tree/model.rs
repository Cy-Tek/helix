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
        self.selected = self
            .selected
            .min(self.visible_entries().len().saturating_sub(1));
        self.generation += 1;
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

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    pub fn toggle_expanded(&mut self, path: &Path) {
        if !self.expanded.insert(path.to_path_buf()) {
            self.expanded.remove(path);
        }
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.contains(path)
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
