use std::{io, path::Path};

use ignore::WalkBuilder;

use super::model::{FileTreeEntry, FileTreeNodeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeLoadOptions {
    pub show_hidden: bool,
    pub follow_symlinks: bool,
    pub parents: bool,
    pub ignore: bool,
    pub git_ignore: bool,
    pub git_global: bool,
    pub git_exclude: bool,
    pub max_depth: Option<usize>,
}

impl Default for TreeLoadOptions {
    fn default() -> Self {
        Self {
            show_hidden: false,
            follow_symlinks: false,
            parents: false,
            ignore: false,
            git_ignore: false,
            git_global: false,
            git_exclude: false,
            max_depth: None,
        }
    }
}

pub fn load_tree_entries(root: &Path, options: &TreeLoadOptions) -> io::Result<Vec<FileTreeEntry>> {
    let mut entries: Vec<FileTreeEntry> = WalkBuilder::new(root)
        .hidden(!options.show_hidden)
        .parents(options.parents)
        .ignore(options.ignore)
        .follow_links(options.follow_symlinks)
        .git_ignore(options.git_ignore)
        .git_global(options.git_global)
        .git_exclude(options.git_exclude)
        .max_depth(options.max_depth.map(|depth| depth + 1))
        .build()
        .filter_map(Result::ok)
        .filter(|entry| entry.path() != root)
        .map(|entry| {
            let path = entry.into_path();
            let kind = node_kind(&path);
            let depth = path
                .strip_prefix(root)
                .ok()
                .map(|relative| relative.components().count().saturating_sub(1))
                .unwrap_or(0);
            FileTreeEntry::new(path, kind, depth)
        })
        .collect();

    entries.sort_by(|left, right| sort_key(root, left).cmp(&sort_key(root, right)));

    Ok(entries)
}

fn node_kind(path: &Path) -> FileTreeNodeKind {
    if path.is_dir() {
        FileTreeNodeKind::Directory
    } else if path.is_symlink() {
        FileTreeNodeKind::Symlink
    } else {
        FileTreeNodeKind::File
    }
}

fn sort_key(root: &Path, entry: &FileTreeEntry) -> (Vec<(usize, String)>, usize) {
    let parts = entry
        .path
        .strip_prefix(root)
        .unwrap_or(&entry.path)
        .components()
        .enumerate()
        .map(|(index, component)| {
            let is_last = index + 1 == entry.depth + 1;
            let kind_order = if is_last && entry.is_dir() { 0 } else { 1 };
            (
                kind_order,
                component.as_os_str().to_string_lossy().to_ascii_lowercase(),
            )
        })
        .collect();
    (parts, entry.depth)
}
