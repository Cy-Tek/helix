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
