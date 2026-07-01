//! The Claude agent panel: a floating window with a session list on the left
//! and the focused session's view (terminal, or — in a later phase — a diff of
//! its edits) on the right.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use helix_core::Position;
use helix_view::agent::{AgentSessionId, AgentStatus, RightPane, WorktreeInfo};
use helix_view::graphics::{CursorKind, Modifier, Rect};
use helix_view::input::{Event, KeyEvent, MouseEvent, MouseEventKind};
use helix_view::keyboard::KeyCode;

use tui::buffer::Buffer as Surface;
use tui::text::Span;
use tui::widgets::{Block, BorderType, Borders, Widget};

use crate::compositor::{Component, Compositor, Context, EventResult};
use crate::job::Callback;
use crate::ui::claude::{spawn_new_session, spawn_session_in, ID};
use crate::ui::spinner::Spinner;
use crate::ui::{terminal, Prompt, PromptEvent};
use crate::{ctrl, key};

/// The floating agent panel. Holds no session state of its own — it reads
/// `editor.agents` afresh each render — only transient view state (cursor,
/// spinner, animation ticker).
pub struct ClaudePanel {
    /// Absolute cursor position computed during the last render, surfaced via
    /// [`Component::cursor`].
    cursor: Option<Position>,
    /// Drives the animated glyph on `Working`/`Starting` rows. Time-based, so a
    /// single shared spinner keeps every animating row in sync.
    spinner: Spinner,
    /// Live only while some session is animating; requests periodic redraws so
    /// the spinner advances even when no input/output events arrive. Dropped
    /// (and its task stopped) when nothing is animating or the panel closes.
    ticker: Option<AnimationTicker>,
    /// Cached patch text for the edits view, keyed by the session it was
    /// computed for. Recomputed when the focused session changes or on `r`.
    diff_cache: Option<(AgentSessionId, String)>,
    /// Scroll offset (in lines) of the edits view.
    diff_scroll: u16,
    /// Grid rect of the focused session's terminal (right pane) from the last
    /// render, used to map absolute mouse coordinates to terminal cells.
    term_grid: Rect,
}

impl ClaudePanel {
    pub fn new() -> Self {
        let mut spinner = Spinner::dots(80);
        spinner.start();
        Self {
            cursor: None,
            spinner,
            ticker: None,
            diff_cache: None,
            diff_scroll: 0,
            term_grid: Rect::default(),
        }
    }
}

/// A background task that calls [`helix_event::request_redraw`] at a fixed
/// cadence so time-based spinners animate while the panel is otherwise idle.
/// The task exits when this guard is dropped.
struct AnimationTicker {
    alive: Arc<AtomicBool>,
}

impl AnimationTicker {
    fn start() -> Self {
        let alive = Arc::new(AtomicBool::new(true));
        let stop = alive.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(100));
            while stop.load(Ordering::Relaxed) {
                interval.tick().await;
                helix_event::request_redraw();
            }
        });
        Self { alive }
    }
}

impl Drop for AnimationTicker {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Relaxed);
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
        let border_style = terminal::brighten(theme.try_get("ui.window").unwrap_or(text_style));
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
        let session_count = ctx.editor.agents.len();

        // Rounded border framing the whole panel, with an inset title that
        // doubles as a focus affordance: it shows the session count and whether
        // keystrokes currently drive the list or the focused terminal.
        let focus_tag = if list_focused { "list" } else { "terminal" };
        let (total_turns, total_cost) =
            ctx.editor.agents.iter().fold((0u32, 0.0f64), |(t, c), s| {
                (
                    t.saturating_add(s.stats.turn_count),
                    c + s.stats.cost_usd,
                )
            });
        let title = if session_count == 0 {
            "─ ◇ Claude Agents ".to_string()
        } else {
            let mut t = format!("─ ◇ Claude Agents · {session_count} · {focus_tag}");
            if total_turns > 0 {
                t.push_str(&format!(" · {total_turns} turns"));
            }
            if total_cost > 0.0 {
                t.push_str(&format!(" · ${total_cost:.2}"));
            }
            t.push(' ');
            t
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(title, title_style));
        let inner = block.inner(area);
        block.render(area, surface);

        // Context-sensitive key hint, cut into the bottom border.
        let viewing_edits = ctx
            .editor
            .agents
            .focused()
            .map(|s| s.right_pane == RightPane::Edits)
            .unwrap_or(false);
        let hint = if ctx.editor.agents.is_empty() {
            "n new · q/esc/C-q close"
        } else if list_focused && viewing_edits {
            "j/k session · C-d/C-u scroll · r refresh · Tab terminal · C-q quit"
        } else if list_focused {
            "j/k select · enter focus · Tab edits · t tab · n new · q close · C-q quit"
        } else {
            "esc/C-o session list · S-esc interrupt · C-q quit session"
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
            self.ticker = None;
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

        type Row = (
            helix_view::agent::AgentSessionId,
            String,
            AgentStatus,
            RightPane,
            Option<String>,
        );
        let rows: Vec<Row> = ctx
            .editor
            .agents
            .iter()
            .map(|s| {
                (
                    s.id,
                    s.display_name.clone(),
                    s.status.clone(),
                    s.right_pane,
                    s.worktree.as_ref().map(|w| w.branch.clone()),
                )
            })
            .collect();

        // Start/stop the redraw ticker so animated rows keep spinning while idle.
        let animating = rows.iter().any(|(_, _, status, ..)| {
            matches!(status, AgentStatus::Working | AgentStatus::Starting)
        });
        match (animating, self.ticker.is_some()) {
            (true, false) => self.ticker = Some(AnimationTicker::start()),
            (false, true) => self.ticker = None,
            _ => {}
        }

        let dim_style = text_style.add_modifier(Modifier::DIM);
        let list_area = inner.with_width(list_width);
        for (i, (id, name, status, _, branch)) in rows.iter().enumerate() {
            let y = list_area.y + i as u16;
            if y >= list_area.bottom() {
                break;
            }
            let is_focused = Some(*id) == focused_id;
            let mut style = if is_focused { selected_style } else { text_style };
            if status.is_awaiting() {
                style = style.patch(attention_style);
            }
            // Animate the glyph for in-flight rows; static symbol otherwise. The
            // two trailing spaces keep the near-full-width symbol off the name.
            let glyph = match status {
                AgentStatus::Working | AgentStatus::Starting => {
                    self.spinner.frame().unwrap_or("●")
                }
                other => status_glyph(other),
            };
            let head = format!("{glyph}  {name}");
            let avail = list_width.saturating_sub(2) as usize;
            surface.set_stringn(list_area.x + 1, y, &head, avail, style);

            // Worktree branch as a dim, right-aligned suffix when it fits.
            if let Some(branch) = branch {
                let suffix = format!("⌥{branch}");
                let suffix_cells = suffix.chars().count() as u16;
                let head_cells = head.chars().count() as u16;
                if list_width > head_cells + suffix_cells + 2 {
                    let sx = list_area.x + list_width - suffix_cells - 1;
                    surface.set_stringn(sx, y, &suffix, suffix_cells as usize, dim_style);
                }
            }
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
            .map(|(_, _, _, pane, _)| *pane)
            .unwrap_or(RightPane::Terminal);

        match right_pane {
            RightPane::Terminal => {
                self.term_grid = content_area;
                // Resize the emulator to the content area, then render its grid.
                if let Some(session) = ctx.editor.agents.focused() {
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
                use crate::ui::claude::diff_view;

                // (Re)compute the patch when the focused session changes.
                let session = focused_id.and_then(|id| {
                    ctx.editor.agents.get(id).map(|s| (id, s.cwd.clone()))
                });
                if let Some((id, cwd)) = session {
                    let stale = self
                        .diff_cache
                        .as_ref()
                        .map(|(cached, _)| *cached != id)
                        .unwrap_or(true);
                    if stale {
                        self.diff_cache = Some((id, diff_view::compute(&cwd)));
                        self.diff_scroll = 0;
                    }
                }

                let mut clamped = self.diff_scroll;
                if let Some((_, text)) = self.diff_cache.as_ref() {
                    let max_scroll =
                        diff_view::line_count(text).saturating_sub(content_area.height.max(1));
                    clamped = self.diff_scroll.min(max_scroll);
                    diff_view::render(text, clamped, content_area, surface, theme);
                }
                self.diff_scroll = clamped;
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
            Event::Mouse(mouse) => return self.handle_mouse(mouse, ctx),
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
    /// The panel owns every mouse event so nothing leaks to the editor behind it
    /// (modal). Wheel scrolls whichever right pane is focused — the edits diff or
    /// the terminal's scrollback. Drag-selection is intercepted upstream by the
    /// compositor, so it never reaches here.
    fn handle_mouse(&mut self, event: &MouseEvent, ctx: &mut Context) -> EventResult {
        let dir = match event.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            _ => return EventResult::Consumed(None),
        };

        let viewing_edits = ctx
            .editor
            .agents
            .focused()
            .map(|s| s.right_pane == RightPane::Edits)
            .unwrap_or(false);
        if viewing_edits {
            // render() clamps diff_scroll to the content, so an over-scroll here
            // is harmless.
            self.diff_scroll = if dir < 0 {
                self.diff_scroll.saturating_sub(3)
            } else {
                self.diff_scroll.saturating_add(3)
            };
        } else if let Some(session) = ctx.editor.agents.focused() {
            // Forward the wheel to the agent's terminal (claude runs a full-screen
            // TUI with mouse reporting, so it scrolls its own transcript).
            let (col, row) = terminal::cell_in(self.term_grid, event);
            session.terminal.wheel(dir < 0, col, row);
        }
        EventResult::Consumed(None)
    }

    fn handle_list_key(&mut self, key: KeyEvent, ctx: &mut Context) -> EventResult {
        // When the focused session is showing its edits diff, these keys scroll
        // it (session navigation with j/k still works for switching sessions).
        let viewing_edits = ctx
            .editor
            .agents
            .focused()
            .map(|s| s.right_pane == RightPane::Edits)
            .unwrap_or(false);
        if viewing_edits {
            match key {
                k if k == ctrl!('d') || k == key!(PageDown) => {
                    self.diff_scroll = self.diff_scroll.saturating_add(15);
                    return EventResult::Consumed(None);
                }
                k if k == ctrl!('u') || k == key!(PageUp) => {
                    self.diff_scroll = self.diff_scroll.saturating_sub(15);
                    return EventResult::Consumed(None);
                }
                key!('r') => {
                    // Force a refresh of the cached diff.
                    self.diff_cache = None;
                    return EventResult::Consumed(None);
                }
                _ => {}
            }
        }

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
                let prompt = worktree_branch_prompt();
                return EventResult::Consumed(Some(Box::new(move |compositor, _| {
                    compositor.push(Box::new(prompt));
                })));
            }
            key!('t') => {
                if let Some(id) = ctx.editor.agents.focused {
                    return EventResult::Consumed(Some(Box::new(move |compositor, cx| {
                        compositor.remove(ID);
                        cx.editor.open_agent_tab(id);
                    })));
                }
            }
            key!(Tab) => {
                if let Some(session) = ctx.editor.agents.focused_mut() {
                    session.right_pane = match session.right_pane {
                        RightPane::Terminal => RightPane::Edits,
                        RightPane::Edits => RightPane::Terminal,
                    };
                }
                // Start the edits view at the top each time it's opened.
                self.diff_scroll = 0;
            }
            // q/Esc dismiss the panel; the sessions keep running in the
            // background (reopenable via the panel or as tabs). Only C-q quits.
            key!('q') => return close_panel(),
            key!(Esc) => return close_panel(),
            k if k == ctrl!('q') => return quit_focused_session(ctx),
            _ => {}
        }
        EventResult::Consumed(None)
    }

    fn handle_terminal_key(&mut self, key: KeyEvent, ctx: &mut Context) -> EventResult {
        // C-q quits the focused session (kills it). Esc / C-o back out to the
        // list non-destructively, so the terminal can never trap the user.
        if key == ctrl!('q') {
            return quit_focused_session(ctx);
        }
        // Escape hatch back to the list (to reach other sessions / new / close).
        if key == ctrl!('o') {
            ctx.editor.agents.list_focused = true;
            return EventResult::Consumed(None);
        }
        // Plain Esc returns to the session list (mirrors a terminal buffer's
        // Esc -> Normal). Shift-Esc (or any modified Esc) sends a real ESC to
        // claude, which is how you actually interrupt/clear it.
        // Plain Esc backs out to the list. Shift-Esc sends a real ESC to claude.
        // Under REPORT_ALTERNATE_KEYS the terminal collapses Shift-Esc to
        // `Char('\u{1b}')` with the SHIFT bit cleared, so treat that (and any
        // modified Esc) as the raw-escape trigger.
        let plain_esc = key.code == KeyCode::Esc && key.modifiers.is_empty();
        let shift_esc = key.code == KeyCode::Char('\u{1b}')
            || (key.code == KeyCode::Esc && !key.modifiers.is_empty());
        if plain_esc {
            ctx.editor.agents.list_focused = true;
            return EventResult::Consumed(None);
        }
        if shift_esc {
            if let Some(session) = ctx.editor.agents.focused() {
                session.terminal.write_escape();
            }
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

/// Quit (terminate) the focused agent session. A worktree-owning session first
/// prompts before deleting its tree. Once no sessions remain, the panel closes.
/// Distinct from [`close_panel`], which only dismisses the panel and leaves
/// every session running.
fn quit_focused_session(ctx: &mut Context) -> EventResult {
    if let Some(id) = ctx.editor.agents.focused {
        // A worktree-owning session asks before deleting the tree.
        let worktree = ctx.editor.agents.get(id).and_then(|s| s.worktree.clone());
        if let Some(info) = worktree {
            let dirty = crate::agent::worktree::is_dirty(&info.path);
            let prompt = worktree_close_prompt(id, info, dirty);
            return EventResult::Consumed(Some(Box::new(move |compositor, _| {
                compositor.push(Box::new(prompt));
            })));
        }
        ctx.editor.agents.remove(id);
    }
    if ctx.editor.agents.is_empty() {
        return close_panel();
    }
    EventResult::Consumed(None)
}

/// Prompt for a branch name, then create a git worktree and spawn an agent in
/// it. Used by the panel's `w` action and `:claude-new-worktree`.
pub(crate) fn worktree_branch_prompt() -> Prompt {
    Prompt::new(
        "new worktree branch: ".into(),
        None,
        crate::ui::completers::none,
        move |cx, input, event| {
            if event != PromptEvent::Validate {
                return;
            }
            let branch = input.trim();
            if branch.is_empty() {
                return;
            }
            match crate::agent::worktree::create(cx.editor, branch) {
                Ok(info) => {
                    let path = info.path.clone();
                    if let Err(err) = spawn_session_in(
                        cx.editor,
                        Some(branch.to_string()),
                        path,
                        Some(info),
                        None,
                    ) {
                        cx.editor.set_error(err.to_string());
                    }
                }
                Err(err) => cx.editor.set_error(format!("Worktree create failed: {err}")),
            }
        },
    )
}

/// Confirm whether to delete an owned worktree when closing its session. The
/// session is always closed; only the on-disk worktree is conditionally removed
/// (force-removed only on explicit confirmation, even when dirty).
pub(crate) fn worktree_close_prompt(id: AgentSessionId, info: WorktreeInfo, dirty: bool) -> Prompt {
    let label = if dirty {
        format!(
            "worktree '{}' has uncommitted changes — delete it? [y/N]: ",
            info.branch
        )
    } else {
        format!("delete worktree '{}'? [y/N]: ", info.branch)
    };
    Prompt::new(
        label.into(),
        None,
        crate::ui::completers::none,
        move |cx, input, event| {
            if event != PromptEvent::Validate {
                return;
            }
            // Closing the session is unconditional; the prompt only governs the
            // worktree directory.
            cx.editor.agents.remove(id);
            let delete = matches!(input.trim(), "y" | "Y" | "yes" | "YES");
            if delete {
                match crate::agent::worktree::remove(&info, dirty) {
                    Ok(()) => cx
                        .editor
                        .set_status(format!("Removed worktree '{}'", info.branch)),
                    Err(err) => cx
                        .editor
                        .set_error(format!("Failed to remove worktree: {err}")),
                }
            } else {
                cx.editor
                    .set_status(format!("Kept worktree at {}", info.path.display()));
            }
            // Close the panel if that was the last session.
            if cx.editor.agents.is_empty() {
                cx.jobs.callback(async {
                    Ok(Callback::EditorCompositor(Box::new(
                        |_editor, compositor: &mut Compositor| {
                            compositor.remove(ID);
                        },
                    )))
                });
            }
        },
    )
}
