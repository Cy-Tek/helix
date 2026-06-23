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
