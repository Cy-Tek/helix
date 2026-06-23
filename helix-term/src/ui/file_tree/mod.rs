#![allow(dead_code)]

use std::path::{Path, PathBuf};

use crate::compositor::{Component, Context, Event, EventResult};
use crate::job::Callback;
use helix_view::{
    editor::Action,
    graphics::{CursorKind, Rect},
};
use tui::buffer::Buffer as Surface;

use self::{
    fs::{load_tree_entries, TreeLoadOptions},
    model::FileTreeModel,
    ops::{FileOperation, FileOperationError, FileOperationService},
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
    preview_provider: preview::FileTreePreviewProvider,
}

#[derive(Debug, Clone, Copy)]
enum OperationPromptKind {
    Create,
    Rename,
    Move,
    Copy,
    Trash,
    ForceDelete,
}

impl FileTree {
    pub fn new(root: PathBuf) -> Self {
        let mut tree = Self {
            model: FileTreeModel::new(root),
            load_options: TreeLoadOptions::default(),
            preview_provider: preview::FileTreePreviewProvider,
        };
        tree.refresh();
        tree
    }

    fn refresh(&mut self) {
        if let Ok(entries) = load_tree_entries(self.model.root(), &self.load_options) {
            self.model.replace_entries(entries);
        }
    }

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

    fn push_operation_prompt(
        &self,
        _cx: &mut Context,
        label: &'static str,
        kind: OperationPromptKind,
    ) -> EventResult {
        let root = self.model.root().to_path_buf();
        let targets = self.model.operation_targets();
        let prompt = super::Prompt::new(
            label.into(),
            None,
            super::completers::none,
            move |cx, input, event| {
                if event != super::PromptEvent::Validate {
                    return;
                }

                let operations = operations_from_prompt(kind, &root, &targets, input);
                if operations.is_empty() {
                    return;
                }

                if let Err(err) = execute_operations(operations) {
                    cx.editor
                        .set_error(format!("File tree operation failed: {err}"));
                    return;
                }

                cx.jobs.callback(async move {
                    let call = Callback::EditorCompositor(Box::new(|editor, compositor| {
                        if let Some(tree) =
                            compositor.find_id::<super::overlay::Overlay<FileTree>>(ID)
                        {
                            tree.content.model.clear_marks();
                            tree.content.refresh();
                        }
                        editor.set_status("File tree operation complete");
                    }));
                    Ok(call)
                });
            },
        );

        EventResult::Consumed(Some(Box::new(move |compositor, _| {
            compositor.push(Box::new(prompt));
        })))
    }

    fn collapse_selected(&mut self) -> EventResult {
        let Some(entry) = self.model.selected_entry().cloned() else {
            return EventResult::Consumed(None);
        };
        if entry.is_dir() && self.model.is_expanded(&entry.path) {
            self.model.toggle_expanded(&entry.path);
        }
        EventResult::Consumed(None)
    }

    fn expand_or_open_selected(&mut self, ctx: &mut Context) -> EventResult {
        let Some(entry) = self.model.selected_entry().cloned() else {
            return EventResult::Consumed(None);
        };

        if entry.is_dir() {
            if !self.model.is_expanded(&entry.path) {
                self.model.toggle_expanded(&entry.path);
            }
            return EventResult::Consumed(None);
        }

        match ctx.editor.open(&entry.path, Action::Replace) {
            Ok(_) => EventResult::Consumed(Some(Box::new(|compositor, _| {
                compositor.remove(ID);
            }))),
            Err(err) => {
                ctx.editor.set_error(format!(
                    "Failed to open file '{}': {err}",
                    entry.path.display()
                ));
                EventResult::Consumed(None)
            }
        }
    }
}

fn execute_operations(operations: Vec<FileOperation>) -> Result<(), FileOperationError> {
    let service = FileOperationService;
    for operation in operations {
        service.execute(operation)?;
    }
    Ok(())
}

fn operations_from_prompt(
    kind: OperationPromptKind,
    root: &Path,
    targets: &[PathBuf],
    input: &str,
) -> Vec<FileOperation> {
    let input = input.trim();
    if input.is_empty() {
        return Vec::new();
    }

    match kind {
        OperationPromptKind::Create => {
            let path = resolve_prompt_path(root, input);
            if input.ends_with(std::path::MAIN_SEPARATOR) {
                vec![FileOperation::CreateDirectory { path }]
            } else {
                vec![FileOperation::CreateFile { path }]
            }
        }
        OperationPromptKind::Rename => targets
            .first()
            .map(|from| {
                let to = resolve_rename_target(root, from, input);
                FileOperation::Rename {
                    from: from.clone(),
                    to,
                }
            })
            .into_iter()
            .collect(),
        OperationPromptKind::Move => move_or_copy_operations(targets, root, input, false),
        OperationPromptKind::Copy => move_or_copy_operations(targets, root, input, true),
        OperationPromptKind::Trash if input == "trash" && !targets.is_empty() => {
            vec![FileOperation::Trash {
                paths: targets.to_vec(),
            }]
        }
        OperationPromptKind::ForceDelete if input == "delete" && !targets.is_empty() => {
            vec![FileOperation::ForceDelete {
                paths: targets.to_vec(),
            }]
        }
        OperationPromptKind::Trash | OperationPromptKind::ForceDelete => Vec::new(),
    }
}

fn move_or_copy_operations(
    targets: &[PathBuf],
    root: &Path,
    input: &str,
    copy: bool,
) -> Vec<FileOperation> {
    let directory = resolve_prompt_path(root, input);
    targets
        .iter()
        .filter_map(|from| {
            let name = from.file_name()?;
            let to = directory.join(name);
            Some(if copy {
                FileOperation::Copy {
                    from: from.clone(),
                    to,
                }
            } else {
                FileOperation::Move {
                    from: from.clone(),
                    to,
                }
            })
        })
        .collect()
}

fn resolve_rename_target(root: &Path, from: &Path, input: &str) -> PathBuf {
    let target = PathBuf::from(input);
    if target.is_absolute() {
        target
    } else {
        from.parent().unwrap_or(root).join(target)
    }
}

fn resolve_prompt_path(root: &Path, input: &str) -> PathBuf {
    let path = PathBuf::from(input);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

impl Component for FileTree {
    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
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
            Some(actions::FileTreeAction::Collapse) => self.collapse_selected(),
            Some(actions::FileTreeAction::ExpandOrOpen) | Some(actions::FileTreeAction::Open) => {
                self.expand_or_open_selected(ctx)
            }
            Some(actions::FileTreeAction::ClearMarks) => {
                self.model.clear_marks();
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
            Some(actions::FileTreeAction::Create) => self.prompt_create(ctx),
            Some(actions::FileTreeAction::Rename) => self.prompt_rename(ctx),
            Some(actions::FileTreeAction::Move) => self.prompt_move(ctx),
            Some(actions::FileTreeAction::Copy) => self.prompt_copy(ctx),
            Some(actions::FileTreeAction::Trash) => self.confirm_trash(ctx),
            Some(actions::FileTreeAction::ForceDelete) => self.confirm_force_delete(ctx),
            Some(actions::FileTreeAction::ShowActions) => {
                let lines = actions::action_labels()
                    .into_iter()
                    .map(|(_, label)| label)
                    .collect::<Vec<_>>()
                    .join("\n");
                let popup = super::Popup::new("file-tree-actions", super::Text::new(lines))
                    .auto_close(true);
                EventResult::Consumed(Some(Box::new(move |compositor, _| {
                    compositor.push(Box::new(popup));
                })))
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
        surface.clear_with(area, ctx.editor.theme.get("ui.background"));
        let layout = render::file_tree_layout(area);
        let tree_area = match layout {
            render::FileTreeLayout::TreeOnly { tree } => tree,
            render::FileTreeLayout::TreeAndPreview { tree, preview } => {
                if let Some(path) = self.model.selected_path() {
                    let inner = super::preview::preview_content_area(preview);
                    if let Some(doc) = ctx.editor.document_by_path(path) {
                        super::preview::render_preview(
                            crate::ui::picker::Preview::EditorDocument(doc),
                            None,
                            preview,
                            surface,
                            ctx.editor,
                            ctx.supports_kitty_graphics,
                            ctx.media,
                        );
                    } else if let Some(file_preview) =
                        self.preview_provider.preview_path_with_loaders(
                            path,
                            inner,
                            ctx.cell_size_pixels,
                            ctx.editor.config.clone(),
                            ctx.editor.syn_loader.clone(),
                        )
                    {
                        let cached = file_preview.into_inner();
                        super::preview::render_preview(
                            crate::ui::picker::Preview::Cached(&cached),
                            None,
                            preview,
                            surface,
                            ctx.editor,
                            ctx.supports_kitty_graphics,
                            ctx.media,
                        );
                    }
                }
                tree
            }
        };
        let rows = self.model.visible_entries();
        render::render_tree_rows(
            surface,
            tree_area,
            &rows,
            self.model.selected_index(),
            |path| self.model.is_expanded(path),
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
