use helix_view::{
    input::KeyEvent,
    keyboard::{KeyCode, KeyModifiers},
};

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
    if event.modifiers != KeyModifiers::NONE {
        return None;
    }

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
        (
            FileTreeAction::Trash,
            "d move selected or marked paths to trash",
        ),
        (
            FileTreeAction::ForceDelete,
            "D permanently delete selected or marked paths",
        ),
        (
            FileTreeAction::ToggleMark,
            "space mark or unmark selected path",
        ),
        (FileTreeAction::ClearMarks, "u clear marks"),
        (FileTreeAction::ToggleHidden, ". toggle hidden files"),
        (FileTreeAction::Refresh, "R refresh tree"),
    ]
}
