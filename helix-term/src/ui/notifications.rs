//! Rendering for the generic toast-notification queue (`editor.notifications`).
//!
//! Toasts are drawn directly in [`Application::render`](crate::application) after
//! the compositor, so they always sit on top of every layer (editor, pickers,
//! the agent panel). They stack down the top-right corner, are colored by
//! severity, and a one-shot timer drives auto-dismiss of timed toasts.

use std::time::Instant;

use helix_view::editor::Severity;
use helix_view::graphics::{Modifier, Rect, Style};
use helix_view::notifications::Notifications;
use helix_view::theme::Theme;
use helix_view::Editor;

use tui::buffer::Buffer as Surface;
use tui::text::Span;
use tui::widgets::{Block, BorderType, Borders, Widget};

use crate::ui::terminal::draw_border_hint;

const WIDTH: u16 = 48;
const MAX_VISIBLE: usize = 4;
const MAX_BODY_LINES: usize = 3;

/// Drop expired toasts, arm the next auto-dismiss timer, and paint the visible
/// stack in the top-right corner.
pub fn render(editor: &mut Editor, area: Rect, surface: &mut Surface) {
    editor.notifications.retain_unexpired(Instant::now());
    schedule_wake(&mut editor.notifications);

    if editor.notifications.is_empty() || area.width < 24 || area.height < 4 {
        return;
    }

    let theme = &editor.theme;
    let text_style = theme.get("ui.text");
    let popup_bg = theme
        .try_get("ui.popup")
        .unwrap_or_else(|| theme.get("ui.background"));

    let width = WIDTH.min(area.width.saturating_sub(2));
    let inner_width = width.saturating_sub(4) as usize;
    let x = area.right().saturating_sub(width + 1);
    let mut y = area.y + 1;

    let total = editor.notifications.len();
    let mut shown = 0;

    // Newest first, at the top of the stack.
    for notification in editor.notifications.iter().rev() {
        if shown >= MAX_VISIBLE {
            break;
        }
        let border_style = severity_style(theme, notification.severity, text_style);
        let title = match &notification.title {
            Some(t) => format!("─ {t} "),
            None => format!("─ {} ", severity_label(notification.severity)),
        };
        let lines = wrap(&notification.body, inner_width, MAX_BODY_LINES);
        let height = lines.len() as u16 + 2; // + top/bottom border
        if y + height > area.bottom() {
            break;
        }

        let rect = Rect::new(x, y, width, height);
        surface.clear_with(rect, popup_bg);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style)
            .title(Span::styled(title, border_style.add_modifier(Modifier::BOLD)));
        let inner = block.inner(rect);
        block.render(rect, surface);

        for (i, line) in lines.iter().enumerate() {
            surface.set_string_truncated(
                inner.x,
                inner.y + i as u16,
                line,
                inner.width as usize,
                |_| text_style,
                true,
                false,
            );
        }

        // Actionable toasts advertise the universal action key.
        if notification.action.is_some() {
            draw_border_hint(surface, rect, "space n", border_style);
        }

        y += height + 1;
        shown += 1;
    }

    // Overflow indicator for toasts that didn't fit / exceed the cap.
    if total > shown && y < area.bottom() {
        let more = format!("+{} more", total - shown);
        let dim = text_style.add_modifier(Modifier::DIM);
        surface.set_string_truncated(x, y, &more, width as usize, |_| dim, true, false);
    }
}

/// Arm a one-shot timer to redraw when the soonest timed toast expires, so it
/// auto-dismisses even while the editor is otherwise idle. De-duped by
/// `take_pending_wake`, so this spawns at most one timer per distinct deadline.
fn schedule_wake(notifications: &mut Notifications) {
    if let Some(deadline) = notifications.take_pending_wake() {
        let deadline = tokio::time::Instant::from_std(deadline);
        tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            helix_event::request_redraw();
        });
    }
}

fn severity_label(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
        Severity::Hint => "hint",
    }
}

/// Border/title color for a severity: a dedicated `ui.notification.*` key if the
/// theme defines one, else the generic diagnostic scope, else plain text.
fn severity_style(theme: &Theme, severity: Severity, fallback: Style) -> Style {
    let (specific, generic) = match severity {
        Severity::Error => ("ui.notification.error", "error"),
        Severity::Warning => ("ui.notification.warning", "warning"),
        Severity::Info => ("ui.notification.info", "ui.text"),
        Severity::Hint => ("ui.notification.hint", "hint"),
    };
    theme
        .try_get(specific)
        .or_else(|| theme.try_get("ui.notification"))
        .or_else(|| theme.try_get(generic))
        .unwrap_or(fallback)
}

/// Greedy word-wrap into at most `max_lines` lines of `width` columns (by char
/// count). If the text overflows, the last line ends with an ellipsis.
fn wrap(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;

    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        let sep = usize::from(!current.is_empty());
        if current_len + sep + word_len > width && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            current_len = 0;
            if lines.len() == max_lines {
                break;
            }
        }
        if !current.is_empty() {
            current.push(' ');
            current_len += 1;
        }
        // A single word longer than the width is hard-truncated below by the
        // renderer's own truncation; just place it.
        current.push_str(word);
        current_len += word_len;
    }
    if lines.len() < max_lines && !current.is_empty() {
        lines.push(current);
    }

    // Mark truncation: if there's leftover text beyond what we captured.
    let captured: usize = lines.iter().map(|l| l.chars().count()).sum();
    let total: usize = text.chars().filter(|c| !c.is_whitespace()).count();
    if captured < total {
        if let Some(last) = lines.last_mut() {
            // Trim to leave room for the ellipsis, then append it.
            while last.chars().count() >= width && width > 1 {
                last.pop();
            }
            last.push('…');
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_short_text_single_line() {
        assert_eq!(wrap("hello world", 40, 3), vec!["hello world"]);
    }

    #[test]
    fn wrap_breaks_on_words() {
        let lines = wrap("one two three four five", 9, 3);
        assert!(lines.len() >= 2);
        assert!(lines.iter().all(|l| l.chars().count() <= 9));
    }

    #[test]
    fn wrap_truncates_with_ellipsis_past_max_lines() {
        let lines = wrap("aaaa bbbb cccc dddd eeee ffff gggg", 4, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines.last().unwrap().ends_with('…'));
    }
}
