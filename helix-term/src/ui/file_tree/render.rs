use std::path::Path;

use helix_view::{graphics::Rect, theme::Style};
use tui::buffer::Buffer as Surface;

use super::model::{FileTreeEntry, FileTreeNodeKind};

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

pub fn render_tree_rows(
    surface: &mut Surface,
    area: Rect,
    rows: &[&FileTreeEntry],
    selected: usize,
    is_expanded: impl Fn(&Path) -> bool,
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
            FileTreeNodeKind::Directory if is_expanded(&entry.path) => "▾ ",
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
        surface.set_stringn(area.x, y, &text, area.width as usize, style);
    }
}
