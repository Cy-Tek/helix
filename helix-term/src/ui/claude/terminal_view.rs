//! Blitting an embedded terminal's neutral grid snapshot into a Helix
//! [`Surface`], plus translation of Helix key events into terminal input byte
//! sequences. The terminal-emulator crate itself is confined to
//! `helix_view::agent::terminal`; this module only consumes its snapshot.

use helix_core::Position;
use helix_view::agent::AgentSession;
use helix_view::graphics::Rect;
use helix_view::input::KeyEvent;
use helix_view::keyboard::{KeyCode, KeyModifiers};

use tui::buffer::Buffer as Surface;

/// Render the focused session's terminal grid into `area`. The caller is
/// responsible for keeping the emulator sized to `area`. Returns the absolute
/// cursor position when the terminal cursor is visible.
pub fn render(session: &AgentSession, area: Rect, surface: &mut Surface) -> Option<Position> {
    let snapshot = session.terminal.snapshot();
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
        // Unicode is UTF-8 encoded.
        assert_eq!(
            encode_key(&key(KeyCode::Char('é'), KeyModifiers::NONE)),
            Some("é".as_bytes().to_vec())
        );
    }

    #[test]
    fn control_chars() {
        // Ctrl-C => 0x03, Ctrl-A => 0x01.
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
        // Shift-Tab is encoded as the back-tab CSI sequence.
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
        // Ctrl-Right uses the CSI modifier parameter (1 + 4 = 5).
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
