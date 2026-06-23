use std::path::PathBuf;

use helix_view::{
    input::KeyEvent,
    keyboard::{KeyCode, KeyModifiers},
};

use super::actions::{action_for_key, FileTreeAction};
use super::fs::{load_tree_entries, TreeLoadOptions};
use super::git::{parse_porcelain_status, GitBadge};
use super::model::{FileTreeEntry, FileTreeModel, FileTreeNodeKind};
use super::ops::{FileOperation, FileOperationService};
use super::preview::{FileTreePreviewProvider, PreviewKind};
use super::render::{file_tree_layout, FileTreeLayout};
use helix_view::graphics::Rect;

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

#[test]
fn loader_sorts_directories_before_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();

    let entries = load_tree_entries(dir.path(), &TreeLoadOptions::default()).unwrap();
    let names: Vec<_> = entries
        .iter()
        .map(|entry| {
            entry
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
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

#[test]
fn preview_provider_classifies_binary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset.bin");
    std::fs::write(&path, b"\0binary").unwrap();

    let provider = FileTreePreviewProvider;
    let preview = provider.preview_path(&path, None).unwrap();

    assert_eq!(preview.kind(), PreviewKind::Binary);
}

#[test]
fn parses_porcelain_status_into_badges() {
    let output = b" M src/main.rs\0?? assets/new.png\0A  Cargo.toml\0";
    let badges = parse_porcelain_status(PathBuf::from("/project").as_path(), output);

    assert_eq!(badges[&path("/project/src/main.rs")], GitBadge::Modified);
    assert_eq!(
        badges[&path("/project/assets/new.png")],
        GitBadge::Untracked
    );
    assert_eq!(badges[&path("/project/Cargo.toml")], GitBadge::Added);
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn maps_core_file_tree_keys_to_actions() {
    assert_eq!(
        action_for_key(key(KeyCode::Char('a'))),
        Some(FileTreeAction::Create)
    );
    assert_eq!(
        action_for_key(key(KeyCode::Char('r'))),
        Some(FileTreeAction::Rename)
    );
    assert_eq!(
        action_for_key(key(KeyCode::Char('d'))),
        Some(FileTreeAction::Trash)
    );
    assert_eq!(
        action_for_key(key(KeyCode::Char('D'))),
        Some(FileTreeAction::ForceDelete)
    );
    assert_eq!(
        action_for_key(key(KeyCode::Char(' '))),
        Some(FileTreeAction::ToggleMark)
    );
    assert_eq!(
        action_for_key(key(KeyCode::Char('?'))),
        Some(FileTreeAction::ShowActions)
    );
}

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
