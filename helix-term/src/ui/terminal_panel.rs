//! The terminals panel: a floating window with a list of managed standalone
//! terminals on the left (each with its process status) and the focused
//! terminal on the right. Deliberately parallel to the Claude agent panel
//! ([`crate::ui::claude::ClaudePanel`]) — same navigation and modal input, but
//! for arbitrary shell/command terminals instead of agent sessions.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use helix_core::Position;
use helix_view::graphics::{CursorKind, Modifier, Rect};
use helix_view::input::{Event, KeyEvent, MouseEvent, MouseEventKind};
use helix_view::keyboard::KeyCode;
use helix_view::terminal_registry::{TerminalId, TerminalStatus};
use helix_view::Editor;

use tui::buffer::Buffer as Surface;
use tui::text::Span;
use tui::widgets::{Block, BorderType, Borders, Widget};

use crate::compositor::{Component, Context, EventResult};
use crate::ui::spinner::Spinner;
use crate::ui::{terminal, Prompt, PromptEvent};
use crate::{ctrl, key};

/// Stable compositor id for the terminals panel.
pub const ID: &str = "terminals-panel";

/// The floating terminals panel. Holds no terminal state of its own — it reads
/// `editor.terminals` afresh each render — only transient view state.
pub struct TerminalsPanel {
    /// Absolute cursor position from the last render, surfaced via [`Component::cursor`].
    cursor: Option<Position>,
    /// Drives the animated glyph on running rows.
    spinner: Spinner,
    /// Live only while some terminal is running; requests periodic redraws so the
    /// spinner advances even when no input/output events arrive.
    ticker: Option<AnimationTicker>,
    /// Grid rect of the focused terminal (right pane) from the last render, used
    /// to map absolute mouse coordinates to terminal cells.
    term_grid: Rect,
}

impl TerminalsPanel {
    pub fn new() -> Self {
        let mut spinner = Spinner::dots(80);
        spinner.start();
        Self {
            cursor: None,
            spinner,
            ticker: None,
            term_grid: Rect::default(),
        }
    }
}

impl Default for TerminalsPanel {
    fn default() -> Self {
        Self::new()
    }
}

/// A background task that requests a redraw at a fixed cadence so time-based
/// spinners animate while the panel is otherwise idle. Exits when dropped.
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

fn status_glyph(status: TerminalStatus) -> &'static str {
    match status {
        TerminalStatus::NotStarted => "○",
        TerminalStatus::InProgress => "●",
        TerminalStatus::Succeeded => "✓",
        TerminalStatus::Failed => "✗",
    }
}

fn status_label(status: TerminalStatus) -> &'static str {
    match status {
        TerminalStatus::NotStarted => "idle",
        TerminalStatus::InProgress => "running",
        TerminalStatus::Succeeded => "done",
        TerminalStatus::Failed => "failed",
    }
}

/// Spawn a terminal running `args` (a shell when empty), register it as a
/// managed session, and focus it in the list. Shared by the panel's `n`/`c`
/// keys and the `:terminals [cmd]` command.
pub(crate) fn spawn_session(editor: &mut Editor, args: &[String]) -> anyhow::Result<TerminalId> {
    let command = args.join(" ");
    let cwd = helix_core::find_workspace().0;
    let pane = terminal::spawn_terminal(editor, args)?;
    let (handle, name) = pane.into_parts();
    Ok(editor.terminals.register(name, command, cwd, handle))
}

/// Prompt for a command line, then spawn it as a new managed terminal.
fn command_prompt() -> Prompt {
    Prompt::new(
        "terminal command: ".into(),
        None,
        crate::ui::completers::none,
        move |cx, input, event| {
            if event != PromptEvent::Validate {
                return;
            }
            let input = input.trim();
            if input.is_empty() {
                return;
            }
            let args: Vec<String> = input.split_whitespace().map(String::from).collect();
            if let Err(err) = spawn_session(cx.editor, &args) {
                cx.editor.set_error(err.to_string());
            }
        },
    )
}

fn close_panel() -> EventResult {
    EventResult::Consumed(Some(Box::new(|compositor, _| {
        compositor.remove(ID);
    })))
}

/// Quit (kill) the focused terminal. Closes the panel once none remain.
fn quit_focused(ctx: &mut Context) -> EventResult {
    if let Some(id) = ctx.editor.terminals.focused {
        ctx.editor.terminals.remove(id);
    }
    if ctx.editor.terminals.is_empty() {
        return close_panel();
    }
    EventResult::Consumed(None)
}

impl Component for TerminalsPanel {
    fn render(&mut self, area: Rect, surface: &mut Surface, ctx: &mut Context) {
        // Settle each terminal's status from its child's exit state.
        ctx.editor.terminals.refresh_statuses();

        let theme = &ctx.editor.theme;
        let base = theme.get("ui.background");
        let text_style = theme.get("ui.text");
        let selected_style = theme.get("ui.selection");
        let border_style = terminal::brighten(theme.try_get("ui.window").unwrap_or(text_style));
        let title_style = border_style.add_modifier(Modifier::BOLD);
        let ok_style = theme.try_get("ui.text.focus").unwrap_or(text_style);
        let fail_style = theme.try_get("error").unwrap_or(text_style);

        surface.clear_with(area, base);
        if area.width < 4 || area.height < 3 {
            self.cursor = None;
            return;
        }

        let list_focused = ctx.editor.terminals.list_focused;
        let count = ctx.editor.terminals.len();

        let focus_tag = if list_focused { "list" } else { "terminal" };
        let title = if count == 0 {
            "─ ◇ Terminals ".to_string()
        } else {
            format!("─ ◇ Terminals · {count} · {focus_tag} ")
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(title, title_style));
        let inner = block.inner(area);
        block.render(area, surface);

        let hint = if ctx.editor.terminals.is_empty() {
            "n shell · c command · q/esc close"
        } else if list_focused {
            "j/k select · enter focus · n shell · c command · t tab · q close · C-q kill"
        } else {
            "esc/C-o list · C-g send esc · C-q kill terminal"
        };
        terminal::draw_border_hint(surface, area, hint, border_style);

        if ctx.editor.terminals.is_empty() {
            let msg = "No terminals.  Press  n  for a shell,  c  for a command.";
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

        // Left: terminal list. Collect rows first so the immutable borrow of the
        // registry is released before the mutable terminal resize below.
        let list_width = ctx
            .editor
            .config()
            .claude_code
            .list_width
            .min(inner.width.saturating_sub(10))
            .max(12);
        let focused_id = ctx.editor.terminals.focused;

        let rows: Vec<(TerminalId, String, TerminalStatus)> = ctx
            .editor
            .terminals
            .iter()
            .map(|s| (s.id, s.name.clone(), s.status))
            .collect();

        // Start/stop the redraw ticker so running rows keep spinning while idle.
        let animating = rows
            .iter()
            .any(|(_, _, status)| *status == TerminalStatus::InProgress);
        match (animating, self.ticker.is_some()) {
            (true, false) => self.ticker = Some(AnimationTicker::start()),
            (false, true) => self.ticker = None,
            _ => {}
        }

        let dim_style = text_style.add_modifier(Modifier::DIM);
        let list_area = inner.with_width(list_width);
        for (i, (id, name, status)) in rows.iter().enumerate() {
            let y = list_area.y + i as u16;
            if y >= list_area.bottom() {
                break;
            }
            let is_focused = Some(*id) == focused_id;
            let style = if is_focused { selected_style } else { text_style };
            let glyph = if *status == TerminalStatus::InProgress {
                self.spinner.frame().unwrap_or("●")
            } else {
                status_glyph(*status)
            };
            let head = format!("{glyph}  {name}");
            let avail = list_width.saturating_sub(2) as usize;
            surface.set_stringn(list_area.x + 1, y, &head, avail, style);

            // Dim, right-aligned status word when it fits.
            let label = status_label(*status);
            let label_style = match status {
                TerminalStatus::Succeeded => ok_style.add_modifier(Modifier::DIM),
                TerminalStatus::Failed => fail_style,
                _ => dim_style,
            };
            let label_cells = label.chars().count() as u16;
            let head_cells = head.chars().count() as u16;
            if list_width > head_cells + label_cells + 2 {
                let sx = list_area.x + list_width - label_cells - 1;
                surface.set_stringn(sx, y, label, label_cells as usize, label_style);
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

        // Right: the focused terminal's grid.
        self.term_grid = content_area;
        if let Some(session) = ctx.editor.terminals.focused() {
            session
                .terminal
                .resize(content_area.height, content_area.width);
        }
        self.cursor = ctx
            .editor
            .terminals
            .focused()
            .and_then(|session| terminal::render(&session.terminal, content_area, surface));
        // Only show the cursor when the terminal has key focus.
        if list_focused {
            self.cursor = None;
        }
    }

    fn cursor(&self, _area: Rect, _editor: &Editor) -> (Option<Position>, CursorKind) {
        match self.cursor {
            Some(pos) => (Some(pos), CursorKind::Block),
            None => (None, CursorKind::Hidden),
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let key = match event {
            Event::Key(key) => *key,
            Event::Paste(text) => {
                if !ctx.editor.terminals.list_focused {
                    if let Some(session) = ctx.editor.terminals.focused() {
                        session.terminal.write_input(text.as_bytes());
                    }
                }
                return EventResult::Consumed(None);
            }
            Event::Mouse(mouse) => return self.handle_mouse(mouse, ctx),
            _ => return EventResult::Ignored(None),
        };

        if ctx.editor.terminals.list_focused {
            self.handle_list_key(key, ctx)
        } else {
            self.handle_terminal_key(key, ctx)
        }
    }

    fn id(&self) -> Option<&'static str> {
        Some(ID)
    }
}

impl TerminalsPanel {
    /// The panel owns every mouse event so nothing leaks to the editor behind
    /// it. The wheel is forwarded to the focused terminal.
    fn handle_mouse(&mut self, event: &MouseEvent, ctx: &mut Context) -> EventResult {
        let up = match event.kind {
            MouseEventKind::ScrollUp => true,
            MouseEventKind::ScrollDown => false,
            _ => return EventResult::Consumed(None),
        };
        if let Some(session) = ctx.editor.terminals.focused() {
            let (col, row) = terminal::cell_in(self.term_grid, event);
            session.terminal.wheel(up, col, row);
        }
        EventResult::Consumed(None)
    }

    fn handle_list_key(&mut self, key: KeyEvent, ctx: &mut Context) -> EventResult {
        match key {
            key!('j') | key!(Down) => {
                ctx.editor.terminals.focus_next();
            }
            key!('k') | key!(Up) => {
                ctx.editor.terminals.focus_prev();
            }
            key!(Enter) | key!('l') | key!(Right) => {
                if ctx.editor.terminals.focused().is_some() {
                    ctx.editor.terminals.list_focused = false;
                }
            }
            key!('n') => {
                if let Err(err) = spawn_session(ctx.editor, &[]) {
                    ctx.editor.set_error(err.to_string());
                }
            }
            key!('c') => {
                let prompt = command_prompt();
                return EventResult::Consumed(Some(Box::new(move |compositor, _| {
                    compositor.push(Box::new(prompt));
                })));
            }
            key!('t') => {
                if let Some(id) = ctx.editor.terminals.focused {
                    return EventResult::Consumed(Some(Box::new(move |compositor, cx| {
                        compositor.remove(ID);
                        cx.editor.open_terminal_session_tab(id);
                    })));
                }
            }
            key!('q') | key!(Esc) => return close_panel(),
            k if k == ctrl!('q') => return quit_focused(ctx),
            _ => {}
        }
        EventResult::Consumed(None)
    }

    fn handle_terminal_key(&mut self, key: KeyEvent, ctx: &mut Context) -> EventResult {
        // C-q kills the focused terminal. Esc / C-o back out to the list
        // non-destructively, so the terminal can never trap the user.
        if key == ctrl!('q') {
            return quit_focused(ctx);
        }
        if key == ctrl!('o') {
            ctx.editor.terminals.list_focused = true;
            return EventResult::Consumed(None);
        }
        // Plain Esc returns to the list. Ctrl-G sends a real ESC to the child
        // (Shift-Esc is undeliverable through many terminal stacks). A modified
        // or alternate-collapsed Esc is honored too, where it arrives.
        let plain_esc = key.code == KeyCode::Esc && key.modifiers.is_empty();
        let send_esc = key == ctrl!('g')
            || key.code == KeyCode::Char('\u{1b}')
            || (key.code == KeyCode::Esc && !key.modifiers.is_empty());
        if plain_esc {
            ctx.editor.terminals.list_focused = true;
            return EventResult::Consumed(None);
        }
        if send_esc {
            if let Some(session) = ctx.editor.terminals.focused() {
                session.terminal.write_escape();
            }
            return EventResult::Consumed(None);
        }
        if let Some(bytes) = terminal::encode_key(&key) {
            if let Some(session) = ctx.editor.terminals.focused() {
                session.terminal.write_input(&bytes);
            }
        }
        EventResult::Consumed(None)
    }
}
