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
    fn handle_event(&mut self, _event: &Event, _ctx: &mut Context) -> EventResult {
        EventResult::Ignored(None)
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, _ctx: &mut Context) {
        surface.clear_with(area, Default::default());
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
