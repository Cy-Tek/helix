//! The agent panel's edits view: a scrollable, colored unified diff of the
//! changes an agent has made in its working directory.
//!
//! We obtain the diff by shelling out to `git` (tracked changes vs `HEAD`, plus
//! new/untracked files rendered as additions) and color the patch by line kind.
//! This is intentionally simpler than threading per-file [`helix_vcs`] diffs and
//! handles multi-file changes, hunk headers, and new files uniformly.

use std::path::Path;
use std::process::Command;

use helix_view::graphics::{Modifier, Rect, Style};
use helix_view::theme::Theme;

use tui::buffer::Buffer as Surface;

/// Run `git -C <cwd> <args>` and return stdout regardless of exit status (diff
/// exits non-zero precisely when there are differences). Returns `None` only if
/// the process couldn't be spawned.
fn git_stdout(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git").arg("-C").arg(cwd).args(args).output().ok()?;
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Compute the patch text for `cwd`: tracked changes against `HEAD` (falling
/// back to the working-tree diff when there is no commit yet), followed by each
/// untracked file rendered as an all-additions diff.
pub fn compute(cwd: &Path) -> String {
    let mut out = String::new();

    let tracked = git_stdout(cwd, &["--no-pager", "diff", "--no-color", "HEAD"]).unwrap_or_default();
    let tracked = if tracked.trim().is_empty() {
        // No HEAD (fresh repo) or nothing staged-vs-HEAD: show working changes.
        git_stdout(cwd, &["--no-pager", "diff", "--no-color"]).unwrap_or_default()
    } else {
        tracked
    };
    out.push_str(&tracked);

    if let Some(list) = git_stdout(cwd, &["ls-files", "--others", "--exclude-standard"]) {
        for file in list.lines().filter(|l| !l.is_empty()) {
            if let Some(diff) = git_stdout(
                cwd,
                &[
                    "--no-pager",
                    "diff",
                    "--no-color",
                    "--no-index",
                    "--",
                    "/dev/null",
                    file,
                ],
            ) {
                out.push_str(&diff);
            }
        }
    }

    if out.trim().is_empty() {
        "No changes in this worktree yet.".to_string()
    } else {
        out
    }
}

/// Number of lines in the patch, for scroll clamping.
pub fn line_count(text: &str) -> u16 {
    text.lines().count().min(u16::MAX as usize) as u16
}

/// Classify a patch line for coloring.
fn line_style(line: &str, theme: &Theme, base: Style) -> Style {
    let get = |key: &str, fallback: Style| theme.try_get(key).unwrap_or(fallback);
    if line.starts_with("@@") {
        get("diff.delta", base.add_modifier(Modifier::BOLD))
    } else if line.starts_with("diff ")
        || line.starts_with("index ")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("new file")
        || line.starts_with("deleted file")
        || line.starts_with("rename ")
    {
        get("diff.delta", base).add_modifier(Modifier::BOLD)
    } else if line.starts_with('+') {
        get("diff.plus", base)
    } else if line.starts_with('-') {
        get("diff.minus", base)
    } else {
        base
    }
}

/// Render the patch into `area`, starting at line `scroll`.
pub fn render(text: &str, scroll: u16, area: Rect, surface: &mut Surface, theme: &Theme) {
    let base = theme.get("ui.text");
    for (row, line) in text
        .lines()
        .skip(scroll as usize)
        .take(area.height as usize)
        .enumerate()
    {
        let y = area.y + row as u16;
        let style = line_style(line, theme, base);
        surface.set_string_truncated(
            area.x,
            y,
            line,
            area.width as usize,
            |_| style,
            true,
            false,
        );
    }
}
