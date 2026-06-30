//! Embedded terminal UI.
//!
//! Provides the shared rendering and key-encoding used by every embedded
//! terminal (the Claude agent panel and the standalone `:terminal`), plus
//! [`TerminalPane`] — a floating, alacritty-powered terminal that runs an
//! arbitrary command. The terminal-emulator crate itself stays confined to
//! [`helix_view::terminal`]; this module only consumes its snapshot.

use std::path::PathBuf;

use helix_core::Position;
use helix_view::graphics::{CursorKind, Rect};
use helix_view::input::{Event, KeyEvent};
use helix_view::keyboard::{KeyCode, KeyModifiers};
use helix_view::terminal::TerminalHandle;
use helix_view::Editor;

use tui::buffer::Buffer as Surface;

use crate::compositor::{Component, Context, EventResult};
use crate::ctrl;

/// Stable compositor id for the standalone terminal.
pub const ID: &str = "terminal";

/// Blit a terminal's neutral grid snapshot into `area`. The caller is
/// responsible for keeping the emulator sized to `area`. Returns the absolute
/// cursor position when the terminal cursor is visible.
pub fn render(terminal: &TerminalHandle, area: Rect, surface: &mut Surface) -> Option<Position> {
    let snapshot = terminal.snapshot();
    let cols = area.width;
    let rows = area.height;
    let mut buf = [0u8; 4];

    for cell in &snapshot.cells {
        if cell.row >= rows || cell.col >= cols {
            continue;
        }
        let width = cell.width.max(1) as usize;
        if cell.col as usize + width > cols as usize {
            continue;
        }
        surface.set_grapheme(
            area.x + cell.col,
            area.y + cell.row,
            cell.symbol.encode_utf8(&mut buf),
            width,
            cell.style,
        );
    }

    snapshot.cursor.and_then(|(row, col)| {
        if row < rows && col < cols {
            Some(Position::new(
                area.y as usize + row as usize,
                area.x as usize + col as usize,
            ))
        } else {
            None
        }
    })
}

/// Resolve the working directory for a new terminal: the workspace root.
fn terminal_cwd() -> PathBuf {
    helix_core::find_workspace().0
}

/// Spawn a [`TerminalPane`] running `args` (or the configured shell when empty),
/// rooted at the workspace.
pub fn spawn_terminal(editor: &Editor, args: &[String]) -> anyhow::Result<TerminalPane> {
    let (configured_shell, scrollback) = {
        let config = editor.config();
        (
            config.embedded_terminal.shell.clone(),
            config.embedded_terminal.scrollback_lines,
        )
    };

    let (program, prog_args, title) = match args.split_first() {
        Some((first, rest)) => (first.clone(), rest.to_vec(), args.join(" ")),
        None => {
            let shell = configured_shell
                .or_else(|| std::env::var("SHELL").ok())
                .unwrap_or_else(|| String::from("/bin/sh"));
            (shell.clone(), Vec::new(), shell)
        }
    };

    let terminal =
        TerminalHandle::spawn(&program, &prog_args, &[], &terminal_cwd(), 24, 80, scrollback)?;
    Ok(TerminalPane::new(terminal, title))
}

/// A floating terminal running a single command. Closes when the user presses
/// the detach chord, or — once the child has exited — on any key.
pub struct TerminalPane {
    terminal: TerminalHandle,
    title: String,
    cursor: Option<Position>,
    exited: bool,
}

impl TerminalPane {
    pub fn new(terminal: TerminalHandle, title: String) -> Self {
        Self {
            terminal,
            title,
            cursor: None,
            exited: false,
        }
    }
}

impl Component for TerminalPane {
    fn render(&mut self, area: Rect, surface: &mut Surface, ctx: &mut Context) {
        let theme = &ctx.editor.theme;
        let base = theme.get("ui.background");
        let header_style = theme.get("ui.statusline");
        surface.clear_with(area, base);

        if area.height == 0 {
            self.cursor = None;
            return;
        }

        // Header row: title on the left, detach hint on the right.
        let exit_note = match self.terminal.exit_status() {
            Some(code) => {
                self.exited = true;
                format!(" [exited {code}] ")
            }
            None => String::new(),
        };
        let header = format!(" {} {}", self.title, exit_note);
        surface.set_stringn(
            area.x,
            area.y,
            &header,
            area.width as usize,
            header_style,
        );
        let hint = if self.exited {
            "any key: close "
        } else {
            "C-q: close "
        };
        let hint_x = area.right().saturating_sub(hint.len() as u16 + 1);
        if hint_x > area.x + header.len() as u16 {
            surface.set_string(hint_x, area.y, hint, header_style);
        }

        let term_area = area.clip_top(1);
        if term_area.height == 0 || term_area.width == 0 {
            self.cursor = None;
            return;
        }

        self.terminal.resize(term_area.height, term_area.width);
        self.cursor = render(&self.terminal, term_area, surface);
        if self.exited {
            self.cursor = None;
        }
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
        let key = match event {
            Event::Key(key) => *key,
            Event::Paste(text) => {
                if !self.exited {
                    self.terminal.write_input(text.as_bytes());
                }
                return EventResult::Consumed(None);
            }
            _ => return EventResult::Ignored(None),
        };

        // Once the process has exited, the pane is just showing final output;
        // any key dismisses it.
        if self.exited {
            return close();
        }

        // Detach chord — closes the pane (and kills the child on drop).
        if key == ctrl!('q') {
            return close();
        }

        if let Some(bytes) = encode_key(&key) {
            self.terminal.write_input(&bytes);
        }
        EventResult::Consumed(None)
    }

    fn cursor(&self, _area: Rect, _editor: &Editor) -> (Option<Position>, CursorKind) {
        match self.cursor {
            Some(pos) => (Some(pos), CursorKind::Block),
            None => (None, CursorKind::Hidden),
        }
    }

    fn id(&self) -> Option<&'static str> {
        Some(ID)
    }
}

fn close() -> EventResult {
    EventResult::Consumed(Some(Box::new(|compositor, _| {
        compositor.remove(ID);
    })))
}

/// Encode a Helix key event as the byte sequence a terminal application expects
/// on its input. Returns `None` for keys with no terminal encoding.
pub fn encode_key(key: &KeyEvent) -> Option<Vec<u8>> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    // CSI modifier parameter: 1 + bitmask(shift=1, alt=2, ctrl=4).
    let csi_mod = 1 + (shift as u8) + (alt as u8) * 2 + (ctrl as u8) * 4;

    let mut bytes: Vec<u8> = match key.code {
        KeyCode::Char(c) => {
            let mut out = Vec::new();
            if ctrl {
                out.push(ctrl_byte(c));
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            out
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab if shift => return Some(b"\x1b[Z".to_vec()),
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => return Some(arrow_seq(b'A', csi_mod)),
        KeyCode::Down => return Some(arrow_seq(b'B', csi_mod)),
        KeyCode::Right => return Some(arrow_seq(b'C', csi_mod)),
        KeyCode::Left => return Some(arrow_seq(b'D', csi_mod)),
        KeyCode::Home => return Some(arrow_seq(b'H', csi_mod)),
        KeyCode::End => return Some(arrow_seq(b'F', csi_mod)),
        KeyCode::PageUp => return Some(tilde_seq(5, csi_mod)),
        KeyCode::PageDown => return Some(tilde_seq(6, csi_mod)),
        KeyCode::Insert => return Some(tilde_seq(2, csi_mod)),
        KeyCode::Delete => return Some(tilde_seq(3, csi_mod)),
        KeyCode::F(n) => return function_key_seq(n),
        _ => return None,
    };

    // Alt-prefix (ESC) for non-CSI keys.
    if alt {
        let mut out = Vec::with_capacity(bytes.len() + 1);
        out.push(0x1b);
        out.append(&mut bytes);
        return Some(out);
    }
    Some(bytes)
}

fn ctrl_byte(c: char) -> u8 {
    match c {
        'a'..='z' => (c as u8) - b'a' + 1,
        'A'..='Z' => (c as u8) - b'A' + 1,
        '@' | ' ' => 0,
        '[' => 0x1b,
        '\\' => 0x1c,
        ']' => 0x1d,
        '^' => 0x1e,
        '_' | '/' => 0x1f,
        '?' => 0x7f,
        other => other as u8,
    }
}

fn arrow_seq(final_byte: u8, csi_mod: u8) -> Vec<u8> {
    if csi_mod > 1 {
        format!("\x1b[1;{}{}", csi_mod, final_byte as char).into_bytes()
    } else {
        vec![0x1b, b'[', final_byte]
    }
}

fn tilde_seq(number: u8, csi_mod: u8) -> Vec<u8> {
    if csi_mod > 1 {
        format!("\x1b[{};{}~", number, csi_mod).into_bytes()
    } else {
        format!("\x1b[{}~", number).into_bytes()
    }
}

fn function_key_seq(n: u8) -> Option<Vec<u8>> {
    let seq: &[u8] = match n {
        1 => b"\x1bOP",
        2 => b"\x1bOQ",
        3 => b"\x1bOR",
        4 => b"\x1bOS",
        5 => b"\x1b[15~",
        6 => b"\x1b[17~",
        7 => b"\x1b[18~",
        8 => b"\x1b[19~",
        9 => b"\x1b[20~",
        10 => b"\x1b[21~",
        11 => b"\x1b[23~",
        12 => b"\x1b[24~",
        _ => return None,
    };
    Some(seq.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_view::keyboard::KeyCode;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent { code, modifiers }
    }

    #[test]
    fn printable_chars() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(b"a".to_vec())
        );
        assert_eq!(
            encode_key(&key(KeyCode::Char('é'), KeyModifiers::NONE)),
            Some("é".as_bytes().to_vec())
        );
    }

    #[test]
    fn control_chars() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![0x03])
        );
        assert_eq!(
            encode_key(&key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Some(vec![0x01])
        );
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('b'), KeyModifiers::ALT)),
            Some(vec![0x1b, b'b'])
        );
    }

    #[test]
    fn named_keys() {
        assert_eq!(
            encode_key(&key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(vec![b'\r'])
        );
        assert_eq!(
            encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE)),
            Some(vec![0x7f])
        );
        assert_eq!(
            encode_key(&key(KeyCode::Esc, KeyModifiers::NONE)),
            Some(vec![0x1b])
        );
        assert_eq!(
            encode_key(&key(KeyCode::Tab, KeyModifiers::SHIFT)),
            Some(b"\x1b[Z".to_vec())
        );
    }

    #[test]
    fn arrow_keys_plain_and_modified() {
        assert_eq!(
            encode_key(&key(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            encode_key(&key(KeyCode::Right, KeyModifiers::CONTROL)),
            Some(b"\x1b[1;5C".to_vec())
        );
    }

    #[test]
    fn tilde_and_function_keys() {
        assert_eq!(
            encode_key(&key(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(b"\x1b[5~".to_vec())
        );
        assert_eq!(
            encode_key(&key(KeyCode::F(1), KeyModifiers::NONE)),
            Some(b"\x1bOP".to_vec())
        );
        assert_eq!(
            encode_key(&key(KeyCode::F(5), KeyModifiers::NONE)),
            Some(b"\x1b[15~".to_vec())
        );
    }
}
