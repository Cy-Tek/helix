use std::path::PathBuf;
use std::sync::Arc;

use arc_swap::{access::DynAccess, ArcSwap};
use helix_core::syntax;
use helix_view::{
    editor::Config as EditorConfig,
    input::KeyEvent,
    keyboard::{KeyCode, KeyModifiers},
};

use super::actions::{action_for_key, FileTreeAction};
use super::fs::{load_tree_entries, TreeLoadOptions};
use super::git::{parse_porcelain_status, GitBadge};
use super::model::{FileTreeEntry, FileTreeModel, FileTreeNodeKind};
use super::ops::{FileOperation, FileOperationService};
use super::preview::{FileTreePreviewProvider, PreviewKind};
use super::render::{file_tree_layout, file_tree_panel_inner, render_tree_rows, FileTreeLayout};
use helix_view::{graphics::Rect, theme::Style};
use tui::buffer::Buffer as Surface;

fn path(path: &str) -> PathBuf {
    PathBuf::from(path)
}

fn test_syntax_loader() -> syntax::Loader {
    let config = helix_loader::config::default_lang_config();
    syntax::Loader::new(config.try_into().unwrap()).unwrap()
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
fn toggling_expanded_directory_hides_children_again() {
    let mut model = FileTreeModel::new(path("/project"));
    model.replace_entries(vec![
        FileTreeEntry::new(path("/project/src"), FileTreeNodeKind::Directory, 0),
        FileTreeEntry::new(path("/project/src/main.rs"), FileTreeNodeKind::File, 1),
    ]);

    model.toggle_expanded(&path("/project/src"));
    assert!(model.is_expanded(&path("/project/src")));

    model.toggle_expanded(&path("/project/src"));

    assert!(!model.is_expanded(&path("/project/src")));
    assert_eq!(model.visible_paths(), vec![path("/project/src")]);
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
fn operation_targets_use_marks_before_selection() {
    let mut model = FileTreeModel::new(path("/project"));
    model.replace_entries(vec![
        FileTreeEntry::new(path("/project/a.rs"), FileTreeNodeKind::File, 0),
        FileTreeEntry::new(path("/project/b.rs"), FileTreeNodeKind::File, 0),
    ]);

    assert_eq!(model.operation_targets(), vec![path("/project/a.rs")]);

    model.toggle_mark_selected();
    model.select_next();

    assert_eq!(model.operation_targets(), vec![path("/project/a.rs")]);
}

#[test]
fn create_base_uses_selected_directory() {
    let mut model = FileTreeModel::new(path("/project"));
    model.replace_entries(vec![
        FileTreeEntry::new(path("/project/src"), FileTreeNodeKind::Directory, 0),
        FileTreeEntry::new(path("/project/README.md"), FileTreeNodeKind::File, 0),
    ]);

    assert_eq!(model.create_base_directory(), path("/project/src"));
}

#[test]
fn create_base_uses_selected_files_parent() {
    let mut model = FileTreeModel::new(path("/project"));
    model.replace_entries(vec![
        FileTreeEntry::new(path("/project/src"), FileTreeNodeKind::Directory, 0),
        FileTreeEntry::new(path("/project/src/main.rs"), FileTreeNodeKind::File, 1),
    ]);
    model.toggle_expanded(&path("/project/src"));
    model.select_next();

    assert_eq!(model.create_base_directory(), path("/project/src"));
}

#[test]
fn create_operation_resolves_relative_to_create_base() {
    let operations = super::operations_from_prompt(
        super::OperationPromptKind::Create,
        &path("/project"),
        &path("/project/src"),
        &[],
        "ui/mod.rs",
    );

    assert_eq!(
        operations,
        vec![FileOperation::CreateFile {
            path: path("/project/src/ui/mod.rs")
        }]
    );
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
fn file_tree_initial_load_only_reads_root_children() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
    std::fs::write(dir.path().join("src/nested/deep.rs"), "").unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();

    let tree = super::FileTree::new(dir.path().to_path_buf());

    assert_eq!(tree.model.entry_count(), 2);
    assert_eq!(
        tree.model.visible_paths(),
        vec![dir.path().join("src"), dir.path().join("README.md")]
    );
}

#[test]
fn expanding_directory_loads_only_that_directorys_direct_children() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
    std::fs::write(dir.path().join("src/nested/deep.rs"), "").unwrap();
    std::fs::write(dir.path().join("README.md"), "").unwrap();

    let mut tree = super::FileTree::new(dir.path().to_path_buf());
    let src = dir.path().join("src");

    tree.expand_selected_directory();

    assert_eq!(
        tree.model.visible_paths(),
        vec![
            src.clone(),
            dir.path().join("src/nested"),
            dir.path().join("src/main.rs"),
            dir.path().join("README.md")
        ]
    );
    assert_eq!(tree.model.entry_count(), 4);
    assert!(tree.model.children_loaded(&src));
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
fn refreshing_after_hidden_toggle_loads_hidden_entries() {
    let dir = tempfile::tempdir().unwrap();
    let hidden = dir.path().join(".env");
    let visible = dir.path().join("main.rs");
    std::fs::write(&hidden, "").unwrap();
    std::fs::write(&visible, "").unwrap();

    let mut tree = super::FileTree::new(dir.path().to_path_buf());

    assert_eq!(tree.model.visible_paths(), vec![visible.clone()]);

    tree.model.toggle_hidden();
    tree.refresh();

    assert_eq!(tree.model.visible_paths(), vec![hidden, visible]);
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
fn preview_provider_classifies_text_files_as_documents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("README.md");
    std::fs::write(&path, "hello from preview\n").unwrap();
    let config: Arc<dyn DynAccess<EditorConfig>> =
        Arc::new(ArcSwap::from_pointee(EditorConfig::default()));
    let syn_loader = Arc::new(ArcSwap::from_pointee(test_syntax_loader()));

    let provider = FileTreePreviewProvider;
    let preview = provider
        .preview_path_with_loaders(&path, Rect::new(0, 0, 40, 20), None, config, syn_loader)
        .unwrap();

    assert_eq!(preview.kind(), PreviewKind::Document);
}

#[test]
fn preview_provider_detects_syntax_for_known_text_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();
    let config: Arc<dyn DynAccess<EditorConfig>> =
        Arc::new(ArcSwap::from_pointee(EditorConfig::default()));
    let syn_loader = Arc::new(ArcSwap::from_pointee(test_syntax_loader()));

    let provider = FileTreePreviewProvider;
    let preview = provider
        .preview_path_with_loaders(&path, Rect::new(0, 0, 40, 20), None, config, syn_loader)
        .unwrap();

    assert_eq!(preview.kind(), PreviewKind::Document);
    assert!(preview.document_has_syntax());
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
fn action_labels_include_direct_and_destructive_actions() {
    let labels: Vec<_> = super::actions::action_labels()
        .into_iter()
        .map(|(_, label)| label)
        .collect();

    assert!(labels.iter().any(|label| label.contains("trash")));
    assert!(labels
        .iter()
        .any(|label| label.contains("permanently delete")));
    assert!(labels.iter().any(|label| label.contains("create")));
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

#[test]
fn panel_inner_reserves_space_for_outer_border() {
    let inner = file_tree_panel_inner(Rect::new(4, 5, 40, 20));

    assert_eq!(inner, Rect::new(5, 6, 38, 18));
}

fn rendered_line(surface: &Surface, y: u16, width: u16) -> String {
    (0..width)
        .map(|x| surface.get(x, y).unwrap().symbol.as_str())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[test]
fn render_tree_rows_draws_directory_and_nested_file_labels() {
    let entries = [
        FileTreeEntry::new(path("/project/src"), FileTreeNodeKind::Directory, 0),
        FileTreeEntry::new(path("/project/src/main.rs"), FileTreeNodeKind::File, 1),
    ];
    let rows: Vec<_> = entries.iter().collect();
    let mut surface = Surface::empty(Rect::new(0, 0, 32, 2));

    render_tree_rows(
        &mut surface,
        Rect::new(0, 0, 32, 2),
        &rows,
        0,
        |candidate| candidate == path("/project/src").as_path(),
        Style::default(),
        Style::default(),
        Style::default(),
    );

    assert_eq!(rendered_line(&surface, 0, 32), "▾ src");
    assert_eq!(rendered_line(&surface, 1, 32), "    main.rs");
}
