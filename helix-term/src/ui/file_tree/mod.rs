#![allow(dead_code)]

use std::path::PathBuf;

use crate::compositor::{Component, Context, Event, EventResult};
use helix_view::graphics::{CursorKind, Rect};
use tui::buffer::Buffer as Surface;

use self::{
    fs::{load_tree_entries, TreeLoadOptions},
    model::FileTreeModel,
};

pub mod actions;
pub mod fs;
pub mod git;
pub mod model;
pub mod ops;
pub mod preview;
pub mod render;

#[cfg(test)]
mod tests;

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
            Some(actions::FileTreeAction::Close) => {
                EventResult::Consumed(Some(Box::new(|compositor, _| {
                    compositor.remove(ID);
                })))
            }
            _ => EventResult::Ignored(None),
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, ctx: &mut Context) {
        surface.clear_with(area, Default::default());
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
    }

    fn cursor(
        &self,
        _area: Rect,
        _ctx: &helix_view::Editor,
    ) -> (Option<helix_core::Position>, CursorKind) {
        (None, CursorKind::Hidden)
    }

    fn id(&self) -> Option<&'static str> {
        Some(ID)
    }
}
