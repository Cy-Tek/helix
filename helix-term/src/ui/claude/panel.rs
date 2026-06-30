//! The Claude agent panel: a floating window with a session list on the left
//! and the focused session's view (terminal, or — in a later phase — a diff of
//! its edits) on the right.

use helix_core::Position;
use helix_view::agent::{AgentStatus, RightPane};
use helix_view::graphics::{CursorKind, Modifier, Rect};
use helix_view::input::{Event, KeyEvent};

use tui::buffer::Buffer as Surface;
use tui::text::Span;
use tui::widgets::{Block, BorderType, Borders, Widget};

use crate::compositor::{Component, Context, EventResult};
use crate::ui::claude::{spawn_new_session, ID};
use crate::ui::terminal;
use crate::{ctrl, key};

/// The floating agent panel. Holds no session state of its own — it reads
/// `editor.agents` afresh each render — only transient layout/cursor info.
pub struct ClaudePanel {
    /// Absolute cursor position computed during the last render, surfaced via
    /// [`Component::cursor`].
    cursor: Option<Position>,
}

impl ClaudePanel {
    pub fn new() -> Self {
        Self { cursor: None }
    }
}

impl Default for ClaudePanel {
    fn default() -> Self {
        Self::new()
    }
}

fn status_glyph(status: &AgentStatus) -> &'static str {
    match status {
        AgentStatus::Starting => "◐",
        AgentStatus::Working => "●",
        AgentStatus::AwaitingAttention(_) => "!",
        AgentStatus::Done => "✓",
        AgentStatus::Exited(_) => "✗",
    }
}

impl Component for ClaudePanel {
    fn render(&mut self, area: Rect, surface: &mut Surface, ctx: &mut Context) {
        let theme = &ctx.editor.theme;
        let base = theme.get("ui.background");
        let text_style = theme.get("ui.text");
        let selected_style = theme.get("ui.selection");
        let border_style = theme.try_get("ui.window").unwrap_or(text_style);
        let title_style = border_style.add_modifier(Modifier::BOLD);
        let attention_style = {
            let s = theme.try_get("ui.agent.attention");
            s.unwrap_or_else(|| theme.get("warning"))
        };

        surface.clear_with(area, base);
        if area.width < 4 || area.height < 3 {
            self.cursor = None;
            return;
        }

        let list_focused = ctx.editor.agents.list_focused;

        // Rounded border framing the whole panel, with an inset title.
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled("─ ◇ Claude Agents ", title_style));
        let inner = block.inner(area);
        block.render(area, surface);

        // Context-sensitive key hint, cut into the bottom border.
        let hint = if ctx.editor.agents.is_empty() {
            "n new · q/esc/C-q close"
        } else if list_focused {
            "j/k select · enter focus · n new · q close session · C-q quit"
        } else {
            "C-o session list · C-q close panel"
        };
        terminal::draw_border_hint(surface, area, hint, border_style);

        if ctx.editor.agents.is_empty() {
            let msg = "No agents running.  Press  n  to start one.";
            let y = inner.y + inner.height / 2;
            surface.set_string_truncated(
                inner.x + 2,
                y,
                msg,
                inner.width.saturating_sub(4) as usize,
                |_| text_style,
                true,
                false,
            );
            self.cursor = None;
            return;
        }

        // Left: session list. Collect rows first so the immutable borrow of the
        // registry is released before the mutable terminal resize below.
        let list_width = ctx
            .editor
            .config()
            .claude_code
            .list_width
            .min(inner.width.saturating_sub(10))
            .max(12);
        let focused_id = ctx.editor.agents.focused;

        let rows: Vec<(helix_view::agent::AgentSessionId, String, AgentStatus, RightPane)> = ctx
            .editor
            .agents
            .iter()
            .map(|s| (s.id, s.display_name.clone(), s.status.clone(), s.right_pane))
            .collect();

        let list_area = inner.with_width(list_width);
        for (i, (id, name, status, _)) in rows.iter().enumerate() {
            let y = list_area.y + i as u16;
            if y >= list_area.bottom() {
                break;
            }
            let is_focused = Some(*id) == focused_id;
            let mut style = if is_focused { selected_style } else { text_style };
            if status.is_awaiting() {
                style = style.patch(attention_style);
            }
            // Two spaces after the glyph so it doesn't sit squished against the
            // name (these status symbols are drawn near the full cell width).
            let line = format!("{}  {}", status_glyph(status), name);
            surface.set_stringn(
                list_area.x + 1,
                y,
                &line,
                list_width.saturating_sub(2) as usize,
                style,
            );
        }

        // Divider column between list and content, tee-joined to the border.
        let divider_x = inner.x + list_width;
        if divider_x < inner.right() {
            for y in inner.y..inner.bottom() {
                surface.set_string(divider_x, y, "│", border_style);
            }
            surface.set_string(divider_x, area.y, "┬", border_style);
            surface.set_string(divider_x, area.bottom() - 1, "┴", border_style);
        }

        let content_area = inner.clip_left(list_width + 1);
        if content_area.width == 0 || content_area.height == 0 {
            self.cursor = None;
            return;
        }

        let right_pane = rows
            .iter()
            .find(|(id, ..)| Some(*id) == focused_id)
            .map(|(.., pane)| *pane)
            .unwrap_or(RightPane::Terminal);

        match right_pane {
            RightPane::Terminal => {
                // Resize the emulator to the content area, then render its grid.
                if let Some(session) = ctx.editor.agents.focused_mut() {
                    session
                        .terminal
                        .resize(content_area.height, content_area.width);
                }
                self.cursor = ctx
                    .editor
                    .agents
                    .focused()
                    .and_then(|session| terminal::render(&session.terminal, content_area, surface));
                // Only show the cursor when the terminal has key focus.
                if list_focused {
                    self.cursor = None;
                }
            }
            RightPane::Edits => {
                let msg = "Edits view — available in a later phase.  Tab: back to terminal";
                surface.set_string_truncated(
                    content_area.x + 1,
                    content_area.y,
                    msg,
                    content_area.width.saturating_sub(2) as usize,
                    |_| text_style,
                    true,
                    false,
                );
                self.cursor = None;
            }
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let key = match event {
            Event::Key(key) => *key,
            Event::Paste(text) => {
                if !ctx.editor.agents.list_focused {
                    if let Some(session) = ctx.editor.agents.focused() {
                        session.terminal.write_input(text.as_bytes());
                    }
                }
                return EventResult::Consumed(None);
            }
            _ => return EventResult::Ignored(None),
        };

        if ctx.editor.agents.list_focused {
            self.handle_list_key(key, ctx)
        } else {
            self.handle_terminal_key(key, ctx)
        }
    }

    fn cursor(&self, _area: Rect, _editor: &helix_view::Editor) -> (Option<Position>, CursorKind) {
        match self.cursor {
            Some(pos) => (Some(pos), CursorKind::Block),
            None => (None, CursorKind::Hidden),
        }
    }

    fn id(&self) -> Option<&'static str> {
        Some(ID)
    }
}

impl ClaudePanel {
    fn handle_list_key(&mut self, key: KeyEvent, ctx: &mut Context) -> EventResult {
        match key {
            key!('j') | key!(Down) => {
                ctx.editor.agents.focus_next();
            }
            key!('k') | key!(Up) => {
                ctx.editor.agents.focus_prev();
            }
            key!(Enter) | key!('l') | key!(Right) => {
                if ctx.editor.agents.focused().is_some() {
                    ctx.editor.agents.list_focused = false;
                }
            }
            key!('n') => {
                if let Err(err) = spawn_new_session(ctx.editor, None) {
                    ctx.editor.set_error(err.to_string());
                }
            }
            key!('w') => {
                ctx.editor
                    .set_status("Worktree sessions are available in a later phase");
            }
            key!(Tab) => {
                if let Some(session) = ctx.editor.agents.focused_mut() {
                    session.right_pane = match session.right_pane {
                        RightPane::Terminal => RightPane::Edits,
                        RightPane::Edits => RightPane::Terminal,
                    };
                }
            }
            key!('q') => {
                if let Some(id) = ctx.editor.agents.focused {
                    ctx.editor.agents.remove(id);
                }
                if ctx.editor.agents.is_empty() {
                    return close_panel();
                }
            }
            key!(Esc) => return close_panel(),
            k if k == ctrl!('q') => return close_panel(),
            _ => {}
        }
        EventResult::Consumed(None)
    }

    fn handle_terminal_key(&mut self, key: KeyEvent, ctx: &mut Context) -> EventResult {
        // Close the whole panel directly, so the terminal can never trap the
        // user. Mirrors the standalone `:terminal` detach chord (C-q).
        if key == ctrl!('q') {
            return close_panel();
        }
        // Escape hatch back to the list (to reach other sessions / new / close).
        if key == ctrl!('o') {
            ctx.editor.agents.list_focused = true;
            return EventResult::Consumed(None);
        }
        if let Some(bytes) = terminal::encode_key(&key) {
            if let Some(session) = ctx.editor.agents.focused() {
                session.terminal.write_input(&bytes);
            }
        }
        EventResult::Consumed(None)
    }
}

fn close_panel() -> EventResult {
    EventResult::Consumed(Some(Box::new(|compositor, _| {
        compositor.remove(ID);
    })))
}
