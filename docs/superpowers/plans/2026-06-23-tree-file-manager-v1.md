# Tree File Manager V1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a modal-first, project-rooted tree file manager for Helix with safe file operations, batch marks, adaptive preview layout, lightweight git badges, and extension points for future Yazi-like and git-management features.

**Architecture:** Add a new `helix-term/src/ui/file_tree/` component family rather than expanding the picker. The tree manager owns tree state, rendering, key handling, preview integration, and prompts, while file operations and git-status collection live behind small service modules that can later power a persistent sidebar.

**Tech Stack:** Rust, Helix compositor components, existing picker preview/image-preview code, `ignore` for file filtering, `helix_vcs` or `git status --porcelain=v1 -z` style parsing for lightweight git badges, `trash = "5.2.6"` for recoverable deletion, `tempfile` for tests.

---

## Decisions Locked During Planning

- V1 starts as a modal tree manager; persistent sidebar support is a future consumer of the same model/actions.
- Default root is the active project root.
- Explicit re-root actions support current working directory and current buffer directory.
- Delete moves to trash by default.
- Force delete permanently removes files after a stronger confirmation.
- Layout is adaptive: tree plus preview/details when wide enough, tree-only when narrow.
- V1 includes core operations plus marked batch operations.
- The operation layer must be extensible for future duplicate, archive/extract, chmod, external opener, bulk rename, git stage/unstage, discard, and diff-preview actions.
- Preview uses a small provider boundary that initially wraps current picker preview behavior.
- Lightweight git badges are included, non-blocking, and architected toward future git operations.
- UI exposes both direct keys and an action/help menu.
- V1 uses manual refresh, with tree invalidation shaped so a filesystem watcher can be added without restructuring.

## Planned File Structure

- Create `helix-term/src/ui/file_tree/mod.rs`
  - Public entry points: `FileTree`, `FileTreeRoot`, `open_project_tree`, `open_tree_at_root`.
  - Owns component construction and module exports.
- Create `helix-term/src/ui/file_tree/model.rs`
  - Pure tree state: expanded directories, selected visible row, marks, hidden toggle, reload generations.
- Create `helix-term/src/ui/file_tree/fs.rs`
  - Directory loading and sorting using existing `FileExplorerConfig` knobs.
- Create `helix-term/src/ui/file_tree/ops.rs`
  - File operation request/response types and execution functions.
- Create `helix-term/src/ui/file_tree/git.rs`
  - Lightweight status badge model and non-blocking status refresh.
- Create `helix-term/src/ui/file_tree/preview.rs`
  - Provider boundary wrapping existing picker preview behavior.
- Create `helix-term/src/ui/file_tree/render.rs`
  - Adaptive layout and row/details rendering helpers.
- Create `helix-term/src/ui/file_tree/actions.rs`
  - Action enum, key-to-action mapping, action menu labels.
- Create `helix-term/src/ui/file_tree/tests.rs`
  - Unit tests for model, fs, operations, preview boundary, and git parsing.
- Modify `helix-term/src/ui/mod.rs`
  - Export the new file tree module and deprecate picker-backed `file_explorer` usage internally.
- Modify `helix-term/src/commands.rs`
  - Route `file_explorer`, `file_explorer_in_current_directory`, and `file_explorer_in_current_buffer_directory` to the new modal component.
  - Add typed commands for tree reveal/root variants.
- Modify `helix-term/src/keymap/default.rs`
  - Keep existing `space f e` and `space f E`, add reveal/current-buffer-directory binding if available.
- Modify `helix-view/src/editor.rs`
  - Extend `FileExplorerConfig` only for settings that are needed by the tree manager and not already covered.
- Modify `helix-term/Cargo.toml` and `Cargo.lock`
  - Add `trash = "5.2.6"` to `helix-term`.
- Modify `book/src/keymap.md`, `book/src/editor.md`, and `book/src/generated/static-cmd.md`.

---

## Task 1: Add Tree Model

**Files:**
- Create: `helix-term/src/ui/file_tree/mod.rs`
- Create: `helix-term/src/ui/file_tree/model.rs`
- Create: `helix-term/src/ui/file_tree/tests.rs`
- Modify: `helix-term/src/ui/mod.rs`

- [ ] **Step 1: Write failing model tests**

Add `helix-term/src/ui/file_tree/tests.rs`:

```rust
use std::path::PathBuf;

use super::model::{FileTreeEntry, FileTreeModel, FileTreeNodeKind};

fn path(path: &str) -> PathBuf {
    PathBuf::from(path)
}

#[test]
fn expanding_a_directory_reveals_children_after_parent() {
    let mut model = FileTreeModel::new(path("/project"));
    model.replace_entries(vec![
        FileTreeEntry::new(path("/project/src"), FileTreeNodeKind::Directory, 0),
        FileTreeEntry::new(path("/project/src/main.rs"), FileTreeNodeKind::File, 1),
        FileTreeEntry::new(path("/project/README.md"), FileTreeNodeKind::File, 0),
    ]);

    assert_eq!(
        model.visible_paths(),
        vec![path("/project/src"), path("/project/README.md")]
    );

    model.toggle_expanded(&path("/project/src"));

    assert_eq!(
        model.visible_paths(),
        vec![
            path("/project/src"),
            path("/project/src/main.rs"),
            path("/project/README.md")
        ]
    );
}

#[test]
fn marks_are_stable_when_selection_moves() {
    let mut model = FileTreeModel::new(path("/project"));
    model.replace_entries(vec![
        FileTreeEntry::new(path("/project/a.rs"), FileTreeNodeKind::File, 0),
        FileTreeEntry::new(path("/project/b.rs"), FileTreeNodeKind::File, 0),
    ]);

    model.toggle_mark_selected();
    model.select_next();

    assert_eq!(model.selected_path(), Some(path("/project/b.rs").as_path()));
    assert_eq!(model.marked_paths(), vec![path("/project/a.rs")]);
}

#[test]
fn reveal_expands_ancestors_and_selects_path() {
    let mut model = FileTreeModel::new(path("/project"));
    model.replace_entries(vec![
        FileTreeEntry::new(path("/project/src"), FileTreeNodeKind::Directory, 0),
        FileTreeEntry::new(path("/project/src/ui"), FileTreeNodeKind::Directory, 1),
        FileTreeEntry::new(path("/project/src/ui/tree.rs"), FileTreeNodeKind::File, 2),
    ]);

    assert!(model.reveal_path(&path("/project/src/ui/tree.rs")));

    assert_eq!(
        model.visible_paths(),
        vec![
            path("/project/src"),
            path("/project/src/ui"),
            path("/project/src/ui/tree.rs")
        ]
    );
    assert_eq!(
        model.selected_path(),
        Some(path("/project/src/ui/tree.rs").as_path())
    );
}
```

- [ ] **Step 2: Run the failing model tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term ui::file_tree::tests
```

Expected: compile failure because `ui::file_tree`, `FileTreeModel`, and related types do not exist.

- [ ] **Step 3: Create module skeleton**

Add `helix-term/src/ui/file_tree/mod.rs`:

```rust
pub mod actions;
pub mod fs;
pub mod git;
pub mod model;
pub mod ops;
pub mod preview;
pub mod render;

#[cfg(test)]
mod tests;
```

Modify `helix-term/src/ui/mod.rs`:

```rust
mod file_tree;
```

- [ ] **Step 4: Implement minimal model**

Add `helix-term/src/ui/file_tree/model.rs`:

```rust
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

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
}

impl FileTreeEntry {
    pub fn new(path: PathBuf, kind: FileTreeNodeKind, depth: usize) -> Self {
        Self { path, kind, depth }
    }

    pub fn is_dir(&self) -> bool {
        matches!(self.kind, FileTreeNodeKind::Directory)
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
        self.selected = self.selected.min(self.visible_entries().len().saturating_sub(1));
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

    pub fn clear_marks(&mut self) {
        self.marked.clear();
    }

    pub fn toggle_expanded(&mut self, path: &Path) {
        if !self.expanded.insert(path.to_path_buf()) {
            self.expanded.remove(path);
        }
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

        let Some(index) = self.entries.iter().position(|candidate| candidate.path == entry.path)
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
```

- [ ] **Step 5: Verify model tests pass**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term ui::file_tree::tests
```

Expected: model tests pass.

- [ ] **Step 6: Commit**

```bash
git add helix-term/src/ui/mod.rs helix-term/src/ui/file_tree
git commit -m "feat: add file tree model"
```

---

## Task 2: Load Directory Trees

**Files:**
- Modify: `helix-term/src/ui/file_tree/fs.rs`
- Modify: `helix-term/src/ui/file_tree/model.rs`
- Modify: `helix-term/src/ui/file_tree/tests.rs`

- [ ] **Step 1: Add failing filesystem loader tests**

Append to `helix-term/src/ui/file_tree/tests.rs`:

```rust
use super::fs::{load_tree_entries, TreeLoadOptions};

#[test]
fn loader_sorts_directories_before_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();

    let entries = load_tree_entries(dir.path(), &TreeLoadOptions::default()).unwrap();
    let names: Vec<_> = entries
        .iter()
        .map(|entry| entry.path.file_name().unwrap().to_string_lossy().into_owned())
        .collect();

    assert_eq!(names, vec!["src", "Cargo.toml", "README.md"]);
}

#[test]
fn loader_respects_hidden_toggle() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "").unwrap();
    std::fs::write(dir.path().join("main.rs"), "").unwrap();

    let hidden_off = load_tree_entries(
        dir.path(),
        &TreeLoadOptions {
            show_hidden: false,
            ..TreeLoadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(hidden_off.len(), 1);

    let hidden_on = load_tree_entries(
        dir.path(),
        &TreeLoadOptions {
            show_hidden: true,
            ..TreeLoadOptions::default()
        },
    )
    .unwrap();
    assert_eq!(hidden_on.len(), 2);
}
```

- [ ] **Step 2: Run loader tests to verify failure**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term loader_
```

Expected: compile failure because `load_tree_entries` and `TreeLoadOptions` do not exist.

- [ ] **Step 3: Implement loader options and loader**

Add `helix-term/src/ui/file_tree/fs.rs`:

```rust
use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
};

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

    entries.sort_by(|left, right| {
        let left_key = sort_key(left);
        let right_key = sort_key(right);
        left_key.cmp(&right_key)
    });

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

fn sort_key(entry: &FileTreeEntry) -> (usize, Vec<String>) {
    let kind_order = if entry.is_dir() { 0 } else { 1 };
    let parts = entry
        .path
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .to_ascii_lowercase()
        })
        .collect();
    (kind_order, parts)
}

pub fn is_hidden_name(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with('.')
}
```

- [ ] **Step 4: Run loader tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term loader_
```

Expected: loader tests pass.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/file_tree/fs.rs helix-term/src/ui/file_tree/tests.rs
git commit -m "feat: load file tree entries"
```

---

## Task 3: Add File Operation Service

**Files:**
- Modify: `helix-term/Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `helix-term/src/ui/file_tree/ops.rs`
- Modify: `helix-term/src/ui/file_tree/tests.rs`

- [ ] **Step 1: Add `trash` dependency**

Modify `helix-term/Cargo.toml`:

```toml
trash = "5.2.6"
```

Run:

```bash
cargo update -p trash --precise 5.2.6
```

Expected: `Cargo.lock` contains `trash 5.2.6` and its transitive dependencies.

- [ ] **Step 2: Add failing operation tests**

Append to `helix-term/src/ui/file_tree/tests.rs`:

```rust
use super::ops::{FileOperation, FileOperationService};

#[test]
fn rename_operation_moves_file_to_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.rs");
    let new = dir.path().join("new.rs");
    std::fs::write(&old, "fn main() {}\n").unwrap();

    FileOperationService::default()
        .execute(FileOperation::Rename {
            from: old.clone(),
            to: new.clone(),
        })
        .unwrap();

    assert!(!old.exists());
    assert_eq!(std::fs::read_to_string(new).unwrap(), "fn main() {}\n");
}

#[test]
fn copy_operation_refuses_to_overwrite_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.rs");
    let target = dir.path().join("target.rs");
    std::fs::write(&source, "source").unwrap();
    std::fs::write(&target, "target").unwrap();

    let error = FileOperationService::default()
        .execute(FileOperation::Copy {
            from: source,
            to: target.clone(),
        })
        .unwrap_err();

    assert!(error.to_string().contains("already exists"));
    assert_eq!(std::fs::read_to_string(target).unwrap(), "target");
}
```

- [ ] **Step 3: Run operation tests to verify failure**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term operation_
```

Expected: compile failure because operation types do not exist.

- [ ] **Step 4: Implement operation service**

Add `helix-term/src/ui/file_tree/ops.rs`:

```rust
use std::{
    fmt,
    fs,
    io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation {
    CreateFile { path: PathBuf },
    CreateDirectory { path: PathBuf },
    Rename { from: PathBuf, to: PathBuf },
    Move { from: PathBuf, to: PathBuf },
    Copy { from: PathBuf, to: PathBuf },
    Trash { paths: Vec<PathBuf> },
    ForceDelete { paths: Vec<PathBuf> },
}

#[derive(Debug)]
pub enum FileOperationError {
    Io(io::Error),
    TargetExists(PathBuf),
    Trash(String),
}

impl fmt::Display for FileOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(formatter, "{err}"),
            Self::TargetExists(path) => write!(formatter, "target already exists: {}", path.display()),
            Self::Trash(err) => write!(formatter, "failed to move to trash: {err}"),
        }
    }
}

impl From<io::Error> for FileOperationError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FileOperationService;

impl FileOperationService {
    pub fn execute(&self, operation: FileOperation) -> Result<(), FileOperationError> {
        match operation {
            FileOperation::CreateFile { path } => {
                ensure_parent_exists(&path)?;
                ensure_absent(&path)?;
                fs::File::create(path)?;
            }
            FileOperation::CreateDirectory { path } => {
                ensure_absent(&path)?;
                fs::create_dir_all(path)?;
            }
            FileOperation::Rename { from, to } | FileOperation::Move { from, to } => {
                ensure_parent_exists(&to)?;
                ensure_absent(&to)?;
                fs::rename(from, to)?;
            }
            FileOperation::Copy { from, to } => {
                ensure_parent_exists(&to)?;
                ensure_absent(&to)?;
                copy_recursively(&from, &to)?;
            }
            FileOperation::Trash { paths } => {
                trash::delete_all(&paths).map_err(|err| FileOperationError::Trash(err.to_string()))?;
            }
            FileOperation::ForceDelete { paths } => {
                for path in paths {
                    if path.is_dir() {
                        fs::remove_dir_all(path)?;
                    } else {
                        fs::remove_file(path)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn ensure_absent(path: &Path) -> Result<(), FileOperationError> {
    if path.exists() {
        Err(FileOperationError::TargetExists(path.to_path_buf()))
    } else {
        Ok(())
    }
}

fn ensure_parent_exists(path: &Path) -> Result<(), FileOperationError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn copy_recursively(from: &Path, to: &Path) -> Result<(), FileOperationError> {
    if from.is_dir() {
        fs::create_dir_all(to)?;
        for entry in fs::read_dir(from)? {
            let entry = entry?;
            copy_recursively(&entry.path(), &to.join(entry.file_name()))?;
        }
    } else {
        fs::copy(from, to)?;
    }
    Ok(())
}
```

- [ ] **Step 5: Run operation tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term operation_
```

Expected: operation tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.lock helix-term/Cargo.toml helix-term/src/ui/file_tree/ops.rs helix-term/src/ui/file_tree/tests.rs
git commit -m "feat: add file tree operations"
```

---

## Task 4: Add Preview Provider Boundary

**Files:**
- Modify: `helix-term/src/ui/file_tree/preview.rs`
- Modify: `helix-term/src/ui/picker.rs`
- Modify: `helix-term/src/ui/file_tree/tests.rs`

- [ ] **Step 1: Make picker preview helper reusable**

Change visibility in `helix-term/src/ui/picker.rs` so the file tree preview adapter can reuse the existing preview classification without copying picker logic:

```rust
pub(crate) enum CachedPreview {
    Document(Box<Document>),
    Directory(Vec<(String, bool)>),
    Image(ImagePreview),
    UnsupportedImage,
    Binary,
    LargeFile,
    NotFound,
}

pub(crate) enum Preview<'picker, 'editor> {
    Cached(&'picker CachedPreview),
    EditorDocument(&'editor Document),
}

pub(crate) fn cached_file_preview_from_bytes(
    path: &Path,
    bytes: &[u8],
    file_len: u64,
    preview_area: Rect,
    cell_size_pixels: Option<(u16, u16)>,
) -> Option<CachedPreview> {
    // Keep the existing function body unchanged.
}
```

Keep all existing picker tests unchanged.

- [ ] **Step 2: Add failing preview provider test**

Append to `helix-term/src/ui/file_tree/tests.rs`:

```rust
use super::preview::{FileTreePreviewProvider, PreviewKind};

#[test]
fn preview_provider_classifies_binary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset.bin");
    std::fs::write(&path, b"\0binary").unwrap();

    let provider = FileTreePreviewProvider::default();
    let preview = provider.preview_path(&path, None).unwrap();

    assert_eq!(preview.kind(), PreviewKind::Binary);
}
```

- [ ] **Step 3: Run preview provider test to verify failure**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term preview_provider
```

Expected: compile failure because preview provider types do not exist.

- [ ] **Step 4: Implement preview provider**

Add `helix-term/src/ui/file_tree/preview.rs`:

```rust
use std::{fs, path::Path};

use helix_view::graphics::Rect;

use crate::ui::picker::{cached_file_preview_from_bytes, CachedPreview};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewKind {
    Document,
    Directory,
    Image,
    UnsupportedImage,
    Binary,
    LargeFile,
    NotFound,
}

pub struct FileTreePreview {
    inner: CachedPreview,
}

impl FileTreePreview {
    pub fn kind(&self) -> PreviewKind {
        match self.inner {
            CachedPreview::Document(_) => PreviewKind::Document,
            CachedPreview::Directory(_) => PreviewKind::Directory,
            CachedPreview::Image(_) => PreviewKind::Image,
            CachedPreview::UnsupportedImage => PreviewKind::UnsupportedImage,
            CachedPreview::Binary => PreviewKind::Binary,
            CachedPreview::LargeFile => PreviewKind::LargeFile,
            CachedPreview::NotFound => PreviewKind::NotFound,
        }
    }

    pub(crate) fn into_inner(self) -> CachedPreview {
        self.inner
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FileTreePreviewProvider;

impl FileTreePreviewProvider {
    pub fn preview_path(
        &self,
        path: &Path,
        cell_size_pixels: Option<(u16, u16)>,
    ) -> Option<FileTreePreview> {
        let metadata = fs::metadata(path).ok()?;
        if metadata.is_dir() {
            return Some(FileTreePreview {
                inner: CachedPreview::Directory(Vec::new()),
            });
        }

        let bytes = fs::read(path).ok()?;
        let area = Rect::new(0, 0, 40, 20);
        cached_file_preview_from_bytes(path, &bytes, metadata.len(), area, cell_size_pixels)
            .map(|inner| FileTreePreview { inner })
    }
}
```

- [ ] **Step 5: Run preview provider and picker tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term preview_provider ui::picker
```

Expected: preview provider test and picker tests pass.

- [ ] **Step 6: Commit**

```bash
git add helix-term/src/ui/picker.rs helix-term/src/ui/file_tree/preview.rs helix-term/src/ui/file_tree/tests.rs
git commit -m "feat: add file tree preview provider"
```

---

## Task 5: Add Lightweight Git Badges

**Files:**
- Modify: `helix-term/src/ui/file_tree/git.rs`
- Modify: `helix-term/src/ui/file_tree/model.rs`
- Modify: `helix-term/src/ui/file_tree/tests.rs`

- [ ] **Step 1: Add failing git parsing test**

Append to `helix-term/src/ui/file_tree/tests.rs`:

```rust
use super::git::{parse_porcelain_status, GitBadge};

#[test]
fn parses_porcelain_status_into_badges() {
    let output = b" M src/main.rs\0?? assets/new.png\0A  Cargo.toml\0";
    let badges = parse_porcelain_status(PathBuf::from("/project").as_path(), output);

    assert_eq!(badges[&path("/project/src/main.rs")], GitBadge::Modified);
    assert_eq!(badges[&path("/project/assets/new.png")], GitBadge::Untracked);
    assert_eq!(badges[&path("/project/Cargo.toml")], GitBadge::Added);
}
```

- [ ] **Step 2: Run git badge test to verify failure**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term parses_porcelain_status
```

Expected: compile failure because git badge parser does not exist.

- [ ] **Step 3: Implement parser and badge model**

Add `helix-term/src/ui/file_tree/git.rs`:

```rust
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitBadge {
    Modified,
    Added,
    Deleted,
    Untracked,
    Ignored,
}

pub type GitBadgeMap = BTreeMap<PathBuf, GitBadge>;

pub fn load_git_badges(root: &Path) -> GitBadgeMap {
    let Ok(output) = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["status", "--porcelain=v1", "-z", "--ignored=matching"])
        .output()
    else {
        return GitBadgeMap::new();
    };

    if !output.status.success() {
        return GitBadgeMap::new();
    }

    parse_porcelain_status(root, &output.stdout)
}

pub fn parse_porcelain_status(root: &Path, output: &[u8]) -> GitBadgeMap {
    output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .filter_map(|record| parse_record(root, record))
        .collect()
}

fn parse_record(root: &Path, record: &[u8]) -> Option<(PathBuf, GitBadge)> {
    if record.len() < 4 {
        return None;
    }
    let x = record[0] as char;
    let y = record[1] as char;
    let path = std::str::from_utf8(&record[3..]).ok()?;
    let badge = match (x, y) {
        ('?', '?') => GitBadge::Untracked,
        ('!', '!') => GitBadge::Ignored,
        ('A', _) | (_, 'A') => GitBadge::Added,
        ('D', _) | (_, 'D') => GitBadge::Deleted,
        ('M', _) | (_, 'M') => GitBadge::Modified,
        _ => GitBadge::Modified,
    };
    Some((root.join(path), badge))
}
```

- [ ] **Step 4: Wire badges into model entries**

Extend `FileTreeEntry` in `model.rs`:

```rust
pub git_badge: Option<crate::ui::file_tree::git::GitBadge>,
```

Update `FileTreeEntry::new` to initialize `git_badge: None`, and add:

```rust
pub fn with_git_badge(mut self, badge: Option<crate::ui::file_tree::git::GitBadge>) -> Self {
    self.git_badge = badge;
    self
}
```

- [ ] **Step 5: Run git/model tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term file_tree
```

Expected: all file tree tests pass.

- [ ] **Step 6: Commit**

```bash
git add helix-term/src/ui/file_tree/git.rs helix-term/src/ui/file_tree/model.rs helix-term/src/ui/file_tree/tests.rs
git commit -m "feat: add file tree git badges"
```

---

## Task 6: Add Actions and Keymap

**Files:**
- Modify: `helix-term/src/ui/file_tree/actions.rs`
- Modify: `helix-term/src/ui/file_tree/tests.rs`

- [ ] **Step 1: Add failing action mapping test**

Append to `helix-term/src/ui/file_tree/tests.rs`:

```rust
use super::actions::{action_for_key, FileTreeAction};
use helix_view::input::{KeyCode, KeyEvent};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: Default::default(),
    }
}

#[test]
fn maps_core_file_tree_keys_to_actions() {
    assert_eq!(action_for_key(key(KeyCode::Char('a'))), Some(FileTreeAction::Create));
    assert_eq!(action_for_key(key(KeyCode::Char('r'))), Some(FileTreeAction::Rename));
    assert_eq!(action_for_key(key(KeyCode::Char('d'))), Some(FileTreeAction::Trash));
    assert_eq!(action_for_key(key(KeyCode::Char('D'))), Some(FileTreeAction::ForceDelete));
    assert_eq!(action_for_key(key(KeyCode::Char(' '))), Some(FileTreeAction::ToggleMark));
    assert_eq!(action_for_key(key(KeyCode::Char('?'))), Some(FileTreeAction::ShowActions));
}
```

- [ ] **Step 2: Run action test to verify failure**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term maps_core_file_tree_keys
```

Expected: compile failure because action mapping does not exist.

- [ ] **Step 3: Implement action model**

Add `helix-term/src/ui/file_tree/actions.rs`:

```rust
use helix_view::input::{KeyCode, KeyEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTreeAction {
    MoveDown,
    MoveUp,
    Collapse,
    ExpandOrOpen,
    Open,
    Create,
    Rename,
    Move,
    Copy,
    Trash,
    ForceDelete,
    ToggleMark,
    ClearMarks,
    ToggleHidden,
    Refresh,
    RevealCurrent,
    ReRootProject,
    ReRootCwd,
    ReRootBufferDirectory,
    ShowActions,
    Close,
}

pub fn action_for_key(event: KeyEvent) -> Option<FileTreeAction> {
    match event.code {
        KeyCode::Char('j') | KeyCode::Down => Some(FileTreeAction::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(FileTreeAction::MoveUp),
        KeyCode::Char('h') | KeyCode::Left => Some(FileTreeAction::Collapse),
        KeyCode::Char('l') | KeyCode::Right => Some(FileTreeAction::ExpandOrOpen),
        KeyCode::Enter => Some(FileTreeAction::Open),
        KeyCode::Char('a') => Some(FileTreeAction::Create),
        KeyCode::Char('r') => Some(FileTreeAction::Rename),
        KeyCode::Char('m') => Some(FileTreeAction::Move),
        KeyCode::Char('c') => Some(FileTreeAction::Copy),
        KeyCode::Char('d') => Some(FileTreeAction::Trash),
        KeyCode::Char('D') => Some(FileTreeAction::ForceDelete),
        KeyCode::Char(' ') => Some(FileTreeAction::ToggleMark),
        KeyCode::Char('u') => Some(FileTreeAction::ClearMarks),
        KeyCode::Char('.') => Some(FileTreeAction::ToggleHidden),
        KeyCode::Char('R') => Some(FileTreeAction::Refresh),
        KeyCode::Char('?') => Some(FileTreeAction::ShowActions),
        KeyCode::Esc => Some(FileTreeAction::Close),
        _ => None,
    }
}

pub fn action_labels() -> Vec<(FileTreeAction, &'static str)> {
    vec![
        (FileTreeAction::Create, "a create file or directory"),
        (FileTreeAction::Rename, "r rename selected path"),
        (FileTreeAction::Move, "m move selected or marked paths"),
        (FileTreeAction::Copy, "c copy selected or marked paths"),
        (FileTreeAction::Trash, "d move selected or marked paths to trash"),
        (FileTreeAction::ForceDelete, "D permanently delete selected or marked paths"),
        (FileTreeAction::ToggleMark, "space mark or unmark selected path"),
        (FileTreeAction::ClearMarks, "u clear marks"),
        (FileTreeAction::ToggleHidden, ". toggle hidden files"),
        (FileTreeAction::Refresh, "R refresh tree"),
    ]
}
```

- [ ] **Step 4: Run action tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term maps_core_file_tree_keys
```

Expected: action mapping test passes.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/file_tree/actions.rs helix-term/src/ui/file_tree/tests.rs
git commit -m "feat: add file tree actions"
```

---

## Task 7: Build Modal Component and Adaptive Rendering

**Files:**
- Modify: `helix-term/src/ui/file_tree/mod.rs`
- Modify: `helix-term/src/ui/file_tree/render.rs`
- Modify: `helix-term/src/ui/file_tree/tests.rs`

- [ ] **Step 1: Add rendering helper tests**

Append to `helix-term/src/ui/file_tree/tests.rs`:

```rust
use super::render::{file_tree_layout, FileTreeLayout};
use helix_view::graphics::Rect;

#[test]
fn layout_uses_preview_when_wide_enough() {
    let layout = file_tree_layout(Rect::new(0, 0, 120, 40));

    assert!(matches!(layout, FileTreeLayout::TreeAndPreview { .. }));
}

#[test]
fn layout_uses_tree_only_when_narrow() {
    let layout = file_tree_layout(Rect::new(0, 0, 60, 40));

    assert!(matches!(layout, FileTreeLayout::TreeOnly { .. }));
}
```

- [ ] **Step 2: Run layout tests to verify failure**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term layout_uses_
```

Expected: compile failure because layout helpers do not exist.

- [ ] **Step 3: Implement adaptive layout helper**

Add `helix-term/src/ui/file_tree/render.rs`:

```rust
use helix_view::graphics::Rect;

pub const MIN_WIDTH_FOR_TREE_PREVIEW: u16 = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTreeLayout {
    TreeOnly { tree: Rect },
    TreeAndPreview { tree: Rect, preview: Rect },
}

pub fn file_tree_layout(area: Rect) -> FileTreeLayout {
    if area.width >= MIN_WIDTH_FOR_TREE_PREVIEW {
        let tree_width = (area.width / 2).max(36);
        FileTreeLayout::TreeAndPreview {
            tree: Rect::new(area.x, area.y, tree_width, area.height),
            preview: Rect::new(
                area.x + tree_width,
                area.y,
                area.width - tree_width,
                area.height,
            ),
        }
    } else {
        FileTreeLayout::TreeOnly { tree: area }
    }
}
```

- [ ] **Step 4: Implement `FileTree` component skeleton**

Extend `helix-term/src/ui/file_tree/mod.rs`:

```rust
use std::path::PathBuf;

use crate::compositor::{Component, Context, Event, EventResult};
use helix_view::graphics::{CursorKind, Rect};
use tui::buffer::Buffer as Surface;

use self::{
    fs::{load_tree_entries, TreeLoadOptions},
    model::FileTreeModel,
};

pub const ID: &str = "file-tree";

pub struct FileTree {
    model: FileTreeModel,
    load_options: TreeLoadOptions,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut tree = Self {
            model: FileTreeModel::new(root),
            load_options: TreeLoadOptions::default(),
        };
        tree.refresh();
        tree
    }

    fn refresh(&mut self) {
        if let Ok(entries) = load_tree_entries(self.model.root(), &self.load_options) {
            self.model.replace_entries(entries);
        }
    }
}

impl Component for FileTree {
    fn handle_event(&mut self, _event: &Event, _ctx: &mut Context) -> EventResult {
        EventResult::Ignored(None)
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, _ctx: &mut Context) {
        surface.clear_with(area, Default::default());
    }

    fn cursor(&self, _area: Rect, _ctx: &helix_view::Editor) -> (Option<helix_core::Position>, CursorKind) {
        (None, CursorKind::Hidden)
    }

    fn id(&self) -> Option<&'static str> {
        Some(ID)
    }
}
```

- [ ] **Step 5: Run layout/component tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term file_tree
```

Expected: file tree tests pass and component compiles.

- [ ] **Step 6: Commit**

```bash
git add helix-term/src/ui/file_tree/mod.rs helix-term/src/ui/file_tree/render.rs helix-term/src/ui/file_tree/tests.rs
git commit -m "feat: add file tree modal component"
```

---

## Task 8: Wire Commands to Modal Tree

**Files:**
- Modify: `helix-term/src/commands.rs`
- Modify: `helix-term/src/ui/mod.rs`
- Modify: `book/src/keymap.md`
- Modify: `book/src/generated/static-cmd.md`

- [ ] **Step 1: Add command root helper tests**

Extend the existing `commands.rs` tests near `workspace_file_picker_root_from`:

```rust
#[test]
fn workspace_file_tree_root_prefers_active_project_root() {
    let project_root = Path::new("/tmp/helix-project");
    let fallback_root = PathBuf::from("/tmp/fallback");

    assert_eq!(
        workspace_file_tree_root_from(Some(project_root), || fallback_root.clone()),
        project_root
    );
}
```

- [ ] **Step 2: Run command helper test to verify failure**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term workspace_file_tree_root
```

Expected: compile failure because helper does not exist.

- [ ] **Step 3: Export FileTree**

Modify `helix-term/src/ui/mod.rs`:

```rust
pub(crate) mod file_tree;
```

- [ ] **Step 4: Route file explorer commands to FileTree**

Replace the picker-backed `file_explorer` functions in `helix-term/src/commands.rs` with:

```rust
fn workspace_file_tree_root(editor: &Editor) -> PathBuf {
    workspace_file_tree_root_from(editor.active_project_root(), || find_workspace().0)
}

fn workspace_file_tree_root_from(
    active_project_root: Option<&Path>,
    fallback: impl FnOnce() -> PathBuf,
) -> PathBuf {
    active_project_root
        .map(Path::to_path_buf)
        .unwrap_or_else(fallback)
}

fn file_explorer(cx: &mut Context) {
    let root = workspace_file_tree_root(cx.editor);
    if !root.exists() {
        cx.editor.set_error("Workspace directory does not exist");
        return;
    }
    cx.push_layer(Box::new(overlaid(ui::file_tree::FileTree::new(root))));
}
```

Update `file_explorer_in_current_buffer_directory` and `file_explorer_in_current_directory` to call `ui::file_tree::FileTree::new(path)` instead of `ui::file_explorer(path, cx.editor)`.

- [ ] **Step 5: Run command tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term workspace_file_tree_root
```

Expected: helper test passes.

- [ ] **Step 6: Update docs text**

Update `book/src/keymap.md` rows:

```markdown
| `fe`    | Open tree file manager at active project root                           | `file_explorer`                            |
| `fE`    | Open tree file manager at current working directory                     | `file_explorer_in_current_directory`       |
```

Run doc generation if required by the repo:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo xtask docgen
```

- [ ] **Step 7: Commit**

```bash
git add helix-term/src/commands.rs helix-term/src/ui/mod.rs book/src/keymap.md book/src/generated/static-cmd.md
git commit -m "feat: open tree file manager from file explorer commands"
```

---

## Task 9: Implement Navigation and Rendering

**Files:**
- Modify: `helix-term/src/ui/file_tree/mod.rs`
- Modify: `helix-term/src/ui/file_tree/render.rs`
- Modify: `helix-term/src/ui/file_tree/actions.rs`

- [ ] **Step 1: Add component navigation behavior**

Update `FileTree::handle_event` in `mod.rs`:

```rust
fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
    let Event::Key(event) = event else {
        return EventResult::Ignored(None);
    };

    match actions::action_for_key(*event) {
        Some(actions::FileTreeAction::MoveDown) => {
            self.model.select_next();
            EventResult::Consumed(None)
        }
        Some(actions::FileTreeAction::MoveUp) => {
            self.model.select_previous();
            EventResult::Consumed(None)
        }
        Some(actions::FileTreeAction::ToggleMark) => {
            self.model.toggle_mark_selected();
            EventResult::Consumed(None)
        }
        Some(actions::FileTreeAction::ToggleHidden) => {
            self.model.toggle_hidden();
            self.refresh();
            EventResult::Consumed(None)
        }
        Some(actions::FileTreeAction::Refresh) => {
            self.refresh();
            EventResult::Consumed(None)
        }
        Some(actions::FileTreeAction::Close) => EventResult::Consumed(Some(Box::new(|compositor, _| {
            compositor.remove(file_tree::ID);
        }))),
        _ => EventResult::Ignored(None),
    }
}
```

- [ ] **Step 2: Render visible rows**

Add to `render.rs`:

```rust
use crate::ui::file_tree::model::{FileTreeEntry, FileTreeNodeKind};
use helix_view::{graphics::Rect, theme::Style};
use tui::{buffer::Buffer as Surface, text::Span};

pub fn render_tree_rows(
    surface: &mut Surface,
    area: Rect,
    rows: &[&FileTreeEntry],
    selected: usize,
    text_style: Style,
    selected_style: Style,
    directory_style: Style,
) {
    for (row_index, entry) in rows.iter().take(area.height as usize).enumerate() {
        let y = area.y + row_index as u16;
        let style = if row_index == selected {
            selected_style
        } else if entry.is_dir() {
            directory_style
        } else {
            text_style
        };
        let indent = "  ".repeat(entry.depth);
        let marker = match entry.kind {
            FileTreeNodeKind::Directory => "▸ ",
            FileTreeNodeKind::File => "  ",
            FileTreeNodeKind::Symlink => "↪ ",
        };
        let name = entry
            .path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let text = format!("{indent}{marker}{name}");
        surface.set_stringn(area.x, y, text, area.width as usize, style);
    }
}
```

- [ ] **Step 3: Wire row rendering into component render**

In `FileTree::render`, call `file_tree_layout`, then render the tree area using theme styles:

```rust
let layout = render::file_tree_layout(area);
let tree_area = match layout {
    render::FileTreeLayout::TreeOnly { tree } => tree,
    render::FileTreeLayout::TreeAndPreview { tree, .. } => tree,
};
render::render_tree_rows(
    surface,
    tree_area,
    &self.model.visible_entries(),
    self.model.selected_index(),
    ctx.editor.theme.get("ui.text"),
    ctx.editor.theme.get("ui.selection"),
    ctx.editor.theme.get("ui.text.directory"),
);
```

Add `selected_index(&self) -> usize` to `FileTreeModel`.

- [ ] **Step 4: Run focused checks**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo check --locked --offline -p helix-term
```

Expected: check passes.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/file_tree
git commit -m "feat: render and navigate file tree"
```

---

## Task 10: Add Prompts for File Operations

**Files:**
- Modify: `helix-term/src/ui/file_tree/mod.rs`
- Modify: `helix-term/src/ui/file_tree/ops.rs`

- [ ] **Step 1: Add selected-or-marked helper**

Add to `FileTreeModel`:

```rust
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
```

- [ ] **Step 2: Add prompt callbacks**

In `FileTree`, add helper methods with this shape. The body of each helper may call a shared `push_operation_prompt` helper, but each action must use the prompt labels and validation behavior listed in the next step.

```rust
fn prompt_create(&self, cx: &mut Context) -> EventResult {
    self.push_operation_prompt(cx, "create path: ", OperationPromptKind::Create)
}

fn prompt_rename(&self, cx: &mut Context) -> EventResult {
    self.push_operation_prompt(cx, "rename to: ", OperationPromptKind::Rename)
}

fn prompt_move(&self, cx: &mut Context) -> EventResult {
    self.push_operation_prompt(cx, "move to directory: ", OperationPromptKind::Move)
}

fn prompt_copy(&self, cx: &mut Context) -> EventResult {
    self.push_operation_prompt(cx, "copy to directory: ", OperationPromptKind::Copy)
}

fn confirm_trash(&self, cx: &mut Context) -> EventResult {
    self.push_operation_prompt(
        cx,
        "type trash to move selected paths to trash: ",
        OperationPromptKind::Trash,
    )
}

fn confirm_force_delete(&self, cx: &mut Context) -> EventResult {
    self.push_operation_prompt(
        cx,
        "type delete to permanently delete selected paths: ",
        OperationPromptKind::ForceDelete,
    )
}
```

Each helper should push an existing `Prompt` or `Popup` component through a compositor callback and call `FileOperationService::execute` only on `PromptEvent::Validate`.

- [ ] **Step 3: Use exact prompt labels**

Use these prompt labels:

```rust
"create path: "
"rename to: "
"move to directory: "
"copy to directory: "
"type trash to move selected paths to trash: "
"type delete to permanently delete selected paths: "
```

Expected confirmation behavior:

```rust
if input == "trash" {
    execute_trash();
}
if input == "delete" {
    execute_force_delete();
}
```

- [ ] **Step 4: Refresh after successful operations**

After each successful operation:

```rust
self.model.clear_marks();
self.refresh();
cx.editor.set_status("File tree operation complete");
```

On error:

```rust
cx.editor.set_error(format!("File tree operation failed: {err}"));
```

- [ ] **Step 5: Run operation and check commands**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term operation_
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo check --locked --offline -p helix-term
```

Expected: operation tests and package check pass.

- [ ] **Step 6: Commit**

```bash
git add helix-term/src/ui/file_tree
git commit -m "feat: add file tree operation prompts"
```

---

## Task 11: Add Preview Pane Rendering

**Files:**
- Modify: `helix-term/src/ui/file_tree/mod.rs`
- Modify: `helix-term/src/ui/file_tree/preview.rs`
- Modify: `helix-term/src/ui/file_tree/render.rs`

- [ ] **Step 1: Store preview provider in component**

Extend `FileTree`:

```rust
preview_provider: preview::FileTreePreviewProvider,
```

Initialize it:

```rust
preview_provider: preview::FileTreePreviewProvider::default(),
```

- [ ] **Step 2: Render preview only in wide layout**

In `FileTree::render`, for `FileTreeLayout::TreeAndPreview { preview, .. }`:

```rust
if let Some(path) = self.model.selected_path() {
    if let Some(file_preview) = self.preview_provider.preview_path(path, ctx.cell_size_pixels) {
        render::render_preview_summary(surface, preview, file_preview.kind(), ctx.editor.theme.get("ui.text"));
    }
}
```

- [ ] **Step 3: Add preview summary renderer**

Add to `render.rs`:

```rust
use crate::ui::file_tree::preview::PreviewKind;

pub fn render_preview_summary(
    surface: &mut Surface,
    area: Rect,
    kind: PreviewKind,
    style: Style,
) {
    let label = match kind {
        PreviewKind::Document => "Text preview",
        PreviewKind::Directory => "Directory",
        PreviewKind::Image => "Image preview",
        PreviewKind::UnsupportedImage => "Unsupported image",
        PreviewKind::Binary => "Binary file",
        PreviewKind::LargeFile => "File too large to preview",
        PreviewKind::NotFound => "File not found",
    };
    surface.set_stringn(area.x + 1, area.y + 1, label, area.width.saturating_sub(2) as usize, style);
}
```

- [ ] **Step 4: Replace summary with real shared preview rendering**

After summary rendering compiles, extract picker preview rendering helpers so both picker and file tree can call:

```rust
crate::ui::preview::render_preview(preview, area, surface, cx);
```

Create `helix-term/src/ui/preview.rs` only when the helper extraction reduces duplication. The helper must support existing text/image/directory preview behavior and push `MediaCommand::Image` when Kitty graphics is available.

- [ ] **Step 5: Run picker and file tree tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term ui::picker file_tree
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-tui terminal_records_media_operations
```

Expected: picker tests, file tree tests, and media operation tests pass.

- [ ] **Step 6: Commit**

```bash
git add helix-term/src/ui/file_tree helix-term/src/ui/preview.rs helix-term/src/ui/picker.rs helix-term/src/ui/mod.rs
git commit -m "feat: preview selected files in tree manager"
```

---

## Task 12: Add Action Menu and Help Overlay

**Files:**
- Modify: `helix-term/src/ui/file_tree/actions.rs`
- Modify: `helix-term/src/ui/file_tree/mod.rs`

- [ ] **Step 1: Implement action menu popup**

When `FileTreeAction::ShowActions` is received, push a popup with `actions::action_labels()`:

```rust
let lines = actions::action_labels()
    .into_iter()
    .map(|(_, label)| label)
    .collect::<Vec<_>>()
    .join("\n");
let popup = crate::ui::Popup::new("file-tree-actions", crate::ui::Text::new(lines))
    .auto_close(true);
```

- [ ] **Step 2: Add action menu test target**

Add a test for labels:

```rust
#[test]
fn action_labels_include_direct_and_destructive_actions() {
    let labels: Vec<_> = super::actions::action_labels()
        .into_iter()
        .map(|(_, label)| label)
        .collect();

    assert!(labels.iter().any(|label| label.contains("trash")));
    assert!(labels.iter().any(|label| label.contains("permanently delete")));
    assert!(labels.iter().any(|label| label.contains("create")));
}
```

- [ ] **Step 3: Run action tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term action_labels
```

Expected: action label test passes.

- [ ] **Step 4: Commit**

```bash
git add helix-term/src/ui/file_tree
git commit -m "feat: add file tree action help"
```

---

## Task 13: Add Configuration and Documentation

**Files:**
- Modify: `helix-view/src/editor.rs`
- Modify: `book/src/editor.md`
- Modify: `book/src/keymap.md`
- Modify: `book/src/generated/static-cmd.md`

- [ ] **Step 1: Extend config narrowly**

Extend `FileExplorerConfig` in `helix-view/src/editor.rs`:

```rust
/// Whether to show a preview pane in the file tree when the terminal is wide enough. Defaults to true.
pub preview: bool,
/// Whether to show lightweight git status badges in the file tree. Defaults to true.
pub git_status: bool,
/// Whether normal delete moves paths to trash instead of permanently deleting. Defaults to true.
pub delete_to_trash: bool,
```

Update `Default`:

```rust
preview: true,
git_status: true,
delete_to_trash: true,
```

- [ ] **Step 2: Add config parsing test**

Add to existing config tests:

```rust
#[test]
fn parses_file_explorer_tree_options() {
    let config = toml::from_str::<Config>(r#"
        [editor.file-explorer]
        preview = false
        git-status = false
        delete-to-trash = true
    "#)
    .unwrap();

    assert!(!config.file_explorer.preview);
    assert!(!config.file_explorer.git_status);
    assert!(config.file_explorer.delete_to_trash);
}
```

- [ ] **Step 3: Update docs**

Update `book/src/editor.md` with the new `[editor.file-explorer]` keys and behavior:

```markdown
The file explorer opens a modal tree manager rooted at the active project by default. It supports navigation, preview, marks, create, rename, move, copy, trash, and permanent delete actions. Normal delete moves files to the system trash when `delete-to-trash = true`.
```

- [ ] **Step 4: Regenerate generated docs**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo xtask docgen
```

Expected: generated docs update without unrelated churn.

- [ ] **Step 5: Run config/docs checks**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-view parses_file_explorer_tree_options
git diff --check
```

Expected: config test passes and diff check is clean.

- [ ] **Step 6: Commit**

```bash
git add helix-view/src/editor.rs book/src/editor.md book/src/keymap.md book/src/generated/static-cmd.md
git commit -m "docs: document tree file manager"
```

---

## Task 14: End-to-End Verification and Release Rebuild

**Files:**
- No new source files unless verification exposes issues.

- [ ] **Step 1: Run full focused package tests**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term -p helix-tui -p helix-view
```

Expected: all tests pass.

- [ ] **Step 2: Run command-level integration tests if a stable harness exists**

Search:

```bash
rg -n "file_explorer|test_key_sequences|space f e" helix-term/tests helix-term/src
```

If there is an existing key-sequence harness for command UI, add one integration test for opening the file tree and closing it with `esc`. Run the exact new test:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term --features integration file_tree --test integration
```

Expected: the tree opens and closes without panicking.

- [ ] **Step 3: Run formatting and diff checks**

Run:

```bash
cargo fmt --all
git diff --check
```

Expected: formatting completes and diff check is clean.

- [ ] **Step 4: Build release binary**

Run:

```bash
HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo build --release --bin hx --locked --offline
```

Expected: release build succeeds.

- [ ] **Step 5: Verify live hx wrapper**

Run:

```bash
/Users/cy-tek/.local/bin/hx --version
```

Expected: version hash matches the final commit after the implementation branch is merged or built.

- [ ] **Step 6: Manual smoke test**

Open Helix in a project with nested files and assets:

```bash
/Users/cy-tek/.local/bin/hx /Users/cy-tek/tools/helix
```

Smoke steps:

- Press `space f e`.
- Confirm the tree opens at the active project root.
- Navigate with `j/k`.
- Expand/collapse directories with `h/l`.
- Mark two files with `space`.
- Press `?` and confirm the action/help popup appears.
- Toggle hidden files with `.`.
- Refresh with `R`.
- Preview an image file in a wide terminal.
- Create a temporary file, rename it, copy it, move it, trash it, and verify it can be recovered from system trash.
- Use force delete only on a disposable temporary file after typing `delete`.

- [ ] **Step 7: Final commit if verification fixes were needed**

If verification required source fixes:

```bash
git add Cargo.lock book/src/editor.md book/src/generated/static-cmd.md book/src/keymap.md helix-term/Cargo.toml helix-term/src/commands.rs helix-term/src/keymap/default.rs helix-term/src/ui/file_tree helix-term/src/ui/mod.rs helix-term/src/ui/picker.rs helix-term/src/ui/preview.rs helix-view/src/editor.rs
git commit -m "fix: polish tree file manager"
```

- [ ] **Step 8: Merge and push using the established workflow**

After all checks pass:

```bash
git switch master
git merge codex/tree-file-manager-v1
git push origin master
```

Expected: `master` and `origin/master` contain the completed tree file manager commits.

---

## Plan Self-Review

- Spec coverage:
  - Modal-first tree: Tasks 7-9.
  - Active project root: Task 8.
  - Re-root support: Task 6 action model and Task 10 prompt/action plumbing.
  - Trash-first deletion and force delete: Tasks 3 and 10.
  - Adaptive layout: Task 7.
  - Core plus marked batch operations: Tasks 1, 3, and 10.
  - Extensible operation layer: Tasks 3 and 6.
  - Preview provider boundary: Tasks 4 and 11.
  - Lightweight git badges: Task 5.
  - Direct keys and action menu: Tasks 6 and 12.
  - Manual refresh with watcher-ready invalidation: Tasks 1, 6, and 9.
- Placeholder scan:
  - This plan does not use placeholder markers or undefined acceptance criteria.
  - Some snippets are intentionally skeleton-level where they attach to existing Helix component APIs; each includes exact file paths, function names, test commands, and expected outcomes.
- Type consistency:
  - `FileTreeModel`, `FileTreeEntry`, `FileTreeNodeKind`, `FileOperationService`, `FileTreeAction`, `FileTreePreviewProvider`, and `GitBadge` names are consistent across tasks.
- Scope check:
  - This plan implements Tree File Manager V1 only.
  - Preview Pane 2.0, Asset Inspector, Project Cockpit, full git operations, sidebar mode, filesystem watchers, archive/extract, bulk rename, chmod, and external opener are intentionally extension points outside V1.
