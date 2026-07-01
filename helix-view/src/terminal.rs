//! Embedded terminal emulator wrapper.
//!
//! Spawns a child process (the `claude` CLI) in a pseudo-terminal via
//! [`portable_pty`] and drives an [`alacritty_terminal`] emulator from a
//! dedicated reader thread. The emulator grid lives behind an
//! [`Arc`]`<`[`FairMutex`]`<Term>>` shared with the render path — exactly the
//! shape Alacritty itself uses. The reader thread is the only writer to the
//! grid; the UI render path is a reader. After every parse the reader thread
//! pokes [`helix_event::request_redraw`] so the main loop repaints.
//!
//! This module is intentionally the *only* place that touches the terminal
//! emulator and PTY crates, so a dependency bump is a one-file change.

use std::cell::Cell as StdCell;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::Arc;

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color as AnsiColor, CursorShape, NamedColor, Processor};
use parking_lot::Mutex;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::graphics::{Color, Modifier, Style, UnderlineStyle};

/// Shared, thread-safe writer to the PTY master.
type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Visible grid dimensions handed to the emulator. `total_lines` equals the
/// number of on-screen rows; scrollback history is governed separately by
/// [`TermConfig::scrolling_history`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    pub columns: usize,
    pub screen_lines: usize,
}

impl Dimensions for TerminalSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// Bridges emulator events back to the editor. `PtyWrite` (terminal replies
/// such as cursor-position reports) must be written back to the child;
/// everything else that changes what is on screen triggers a redraw.
#[derive(Clone)]
pub struct EventProxy {
    writer: SharedWriter,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: TermEvent) {
        match event {
            TermEvent::PtyWrite(text) => {
                let mut writer = self.writer.lock();
                let _ = writer.write_all(text.as_bytes());
                let _ = writer.flush();
            }
            TermEvent::Wakeup
            | TermEvent::Bell
            | TermEvent::Title(_)
            | TermEvent::ResetTitle
            | TermEvent::CursorBlinkingChange => {
                helix_event::request_redraw();
            }
            _ => {}
        }
    }
}

/// A live embedded terminal: the emulator grid plus the handles needed to
/// feed it input, resize it, and tear it down.
pub struct TerminalHandle {
    /// The emulator grid, shared with the reader thread. Lock briefly to read
    /// renderable content during rendering.
    term: Arc<FairMutex<Term<EventProxy>>>,
    writer: SharedWriter,
    master: Box<dyn MasterPty + Send>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    size: StdCell<TerminalSize>,
}

impl TerminalHandle {
    /// Spawn `program` with `args` in a PTY rooted at `cwd`, sized
    /// `rows`×`cols`, with `scrollback` lines of history.
    pub fn spawn(
        program: &str,
        args: &[String],
        envs: &[(String, String)],
        cwd: &Path,
        rows: u16,
        cols: u16,
        scrollback: usize,
    ) -> anyhow::Result<Self> {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let size = TerminalSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.cwd(cwd);
        // A sensible default terminal type; claude's TUI honours truecolor.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in envs {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd)?;
        // Drop the slave so the master observes EOF once the child exits.
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let writer: SharedWriter = Arc::new(Mutex::new(pair.master.take_writer()?));

        let config = TermConfig {
            scrolling_history: scrollback,
            ..TermConfig::default()
        };
        let proxy = EventProxy {
            writer: writer.clone(),
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &size, proxy)));

        spawn_reader(reader, term.clone());

        Ok(Self {
            term,
            writer,
            master: pair.master,
            child: Arc::new(Mutex::new(child)),
            size: StdCell::new(size),
        })
    }

    pub fn size(&self) -> TerminalSize {
        self.size.get()
    }

    /// Take a neutral snapshot of the visible grid for rendering. Coordinates
    /// are relative to the grid origin; cell styles are already mapped to Helix
    /// [`Style`]. This keeps the terminal-emulator crate confined to this module.
    pub fn snapshot(&self) -> TerminalSnapshot {
        let term = self.term.lock();
        let content = term.renderable_content();

        let mut cells = Vec::new();
        for indexed in content.display_iter {
            let cell: &Cell = indexed.cell;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            let line = indexed.point.line.0;
            if line < 0 {
                continue;
            }
            let symbol = if cell.c == '\0' || cell.c == '\t' {
                ' '
            } else {
                cell.c
            };
            let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            cells.push(TerminalCell {
                row: line as u16,
                col: indexed.point.column.0 as u16,
                width,
                symbol,
                style: convert_style(cell),
            });
        }

        let cursor = if content.cursor.shape != CursorShape::Hidden {
            let row = content.cursor.point.line.0;
            let col = content.cursor.point.column.0;
            if row >= 0 {
                Some((row as u16, col as u16))
            } else {
                None
            }
        } else {
            None
        };

        TerminalSnapshot { cells, cursor }
    }

    /// React to a mouse-wheel notch at grid cell (`col`, `row`), doing whatever
    /// the running program's current mode calls for — matching how a real
    /// terminal behaves:
    ///
    /// * **Mouse reporting on** (e.g. the `claude` TUI): forward the wheel as a
    ///   mouse event so the app scrolls its own view. Full-screen apps use the
    ///   alternate screen and keep no scrollback of their own, so this is the
    ///   only way to scroll them.
    /// * **Alternate screen, no mouse reporting**: emit arrow keys ("alternate
    ///   scroll"), the conventional fallback for full-screen apps.
    /// * **Normal screen**: scroll our local scrollback buffer.
    pub fn wheel(&self, up: bool, col: u16, row: u16) {
        let mode = *self.term.lock().mode();
        if mode.intersects(TermMode::MOUSE_MODE) {
            self.report_wheel(mode, up, col, row);
        } else if mode.contains(TermMode::ALT_SCREEN) {
            let seq: &[u8] = if up { b"\x1b[A" } else { b"\x1b[B" };
            for _ in 0..3 {
                self.write_input(seq);
            }
        } else {
            let delta = if up { 3 } else { -3 };
            self.term.lock().scroll_display(Scroll::Delta(delta));
            helix_event::request_redraw();
        }
    }

    /// Encode a wheel notch as an X10/SGR mouse event and send it to the child.
    /// Wheel up/down are buttons 64/65; coordinates are 1-based.
    fn report_wheel(&self, mode: TermMode, up: bool, col: u16, row: u16) {
        let button = if up { 64u8 } else { 65u8 };
        let (cx, cy) = (col.saturating_add(1), row.saturating_add(1));
        let seq: Vec<u8> = if mode.contains(TermMode::SGR_MOUSE) {
            format!("\x1b[<{button};{cx};{cy}M").into_bytes()
        } else {
            // Legacy encoding: each field is offset by 32; the addressable range
            // caps at 223 (255 - 32).
            let bx = (cx.min(223) as u8).saturating_add(32);
            let by = (cy.min(223) as u8).saturating_add(32);
            vec![0x1b, b'[', b'M', button + 32, bx, by]
        };
        self.write_input(&seq);
    }

    /// Feed raw bytes (already encoded for the terminal) to the child's stdin.
    pub fn write_input(&self, bytes: &[u8]) {
        let mut writer = self.writer.lock();
        let _ = writer.write_all(bytes);
        let _ = writer.flush();
    }

    /// Resize both the PTY and the emulator. No-op when unchanged.
    pub fn resize(&self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let new = TerminalSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        if new == self.size.get() {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.term.lock().resize(new);
        self.size.set(new);
    }

    /// True once the child process has exited.
    pub fn exit_status(&self) -> Option<i32> {
        self.child
            .lock()
            .try_wait()
            .ok()
            .flatten()
            .map(|status| status.exit_code() as i32)
    }

    /// Kill the child process. Idempotent.
    pub fn kill(&self) {
        let _ = self.child.lock().kill();
    }
}

impl Drop for TerminalHandle {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A single rendered grid cell, in neutral terms. Coordinates are relative to
/// the grid origin (the caller offsets them into its render area).
pub struct TerminalCell {
    pub row: u16,
    pub col: u16,
    pub width: u8,
    pub symbol: char,
    pub style: Style,
}

/// A point-in-time view of the terminal grid for rendering.
pub struct TerminalSnapshot {
    pub cells: Vec<TerminalCell>,
    /// Cursor position (row, col), relative to the grid origin, when visible.
    pub cursor: Option<(u16, u16)>,
}

fn convert_style(cell: &Cell) -> Style {
    let mut style = Style::default();
    style.fg = convert_color(cell.fg);
    style.bg = convert_color(cell.bg);

    let flags = cell.flags;
    let mut modifier = Modifier::empty();
    if flags.contains(Flags::BOLD) {
        modifier |= Modifier::BOLD;
    }
    if flags.contains(Flags::DIM) {
        modifier |= Modifier::DIM;
    }
    if flags.contains(Flags::ITALIC) {
        modifier |= Modifier::ITALIC;
    }
    if flags.contains(Flags::INVERSE) {
        modifier |= Modifier::REVERSED;
    }
    if flags.contains(Flags::HIDDEN) {
        modifier |= Modifier::HIDDEN;
    }
    if flags.contains(Flags::STRIKEOUT) {
        modifier |= Modifier::CROSSED_OUT;
    }
    style.add_modifier = modifier;

    if flags.intersects(Flags::ALL_UNDERLINES) {
        style.underline_style = Some(UnderlineStyle::Line);
    }
    style
}

/// Map an alacritty color to a Helix color. Named/indexed colors pass through as
/// palette indices so the embedded terminal inherits the outer terminal's
/// palette; the default fg/bg map to `None` (the surrounding theme's default).
fn convert_color(color: AnsiColor) -> Option<Color> {
    match color {
        AnsiColor::Spec(rgb) => Some(Color::Rgb(rgb.r, rgb.g, rgb.b)),
        AnsiColor::Indexed(i) => Some(Color::Indexed(i)),
        AnsiColor::Named(named) => named_to_index(named).map(Color::Indexed),
    }
}

fn named_to_index(named: NamedColor) -> Option<u8> {
    use NamedColor::*;
    Some(match named {
        Black | DimBlack => 0,
        Red | DimRed => 1,
        Green | DimGreen => 2,
        Yellow | DimYellow => 3,
        Blue | DimBlue => 4,
        Magenta | DimMagenta => 5,
        Cyan | DimCyan => 6,
        White | DimWhite => 7,
        BrightBlack => 8,
        BrightRed => 9,
        BrightGreen => 10,
        BrightYellow => 11,
        BrightBlue => 12,
        BrightMagenta => 13,
        BrightCyan => 14,
        BrightWhite => 15,
        Foreground | Background | Cursor | BrightForeground | DimForeground => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    /// Exercise the full pipe: spawn a process in a PTY, let the VT parser fill
    /// the grid, and confirm the snapshot reflects the output.
    #[test]
    fn echo_appears_in_snapshot() {
        let handle = TerminalHandle::spawn(
            "sh",
            &["-c".to_string(), "printf HELLO".to_string()],
            &[],
            &PathBuf::from("."),
            24,
            80,
            1000,
        )
        .expect("spawn pty");

        // The reader thread fills the grid asynchronously; poll briefly.
        let deadline = Instant::now() + Duration::from_secs(5);
        let text = loop {
            let snapshot = handle.snapshot();
            let text: String = {
                let mut cells: Vec<_> = snapshot.cells.iter().collect();
                cells.sort_by_key(|c| (c.row, c.col));
                cells.iter().map(|c| c.symbol).collect()
            };
            if text.contains("HELLO") || Instant::now() >= deadline {
                break text;
            }
            std::thread::sleep(Duration::from_millis(25));
        };

        assert!(
            text.contains("HELLO"),
            "expected terminal grid to contain the echoed text, got: {text:?}"
        );
    }
}

/// Spawn the per-session reader thread: blocking PTY reads → VT parser →
/// emulator grid, with a redraw poke after each chunk.
fn spawn_reader(mut reader: Box<dyn Read + Send>, term: Arc<FairMutex<Term<EventProxy>>>) {
    std::thread::spawn(move || {
        let mut parser: Processor = Processor::new();
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    {
                        let mut term = term.lock();
                        parser.advance(&mut *term, &buf[..n]);
                    }
                    helix_event::request_redraw();
                }
            }
        }
        // Final repaint so the exited state is shown.
        helix_event::request_redraw();
    });
}
