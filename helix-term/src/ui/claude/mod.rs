//! Claude Code agent panel UI.
//!
//! A floating window listing managed `claude` sessions (left) with the focused
//! session's embedded terminal on the right. See [`panel::ClaudePanel`].

pub mod panel;
pub mod terminal_view;

pub use panel::ClaudePanel;

use helix_view::agent::{AgentSessionId, SpawnConfig};
use helix_view::Editor;

/// Stable compositor id for the panel layer.
pub const ID: &str = "claude-panel";

/// Spawn a new agent session rooted at the workspace, honouring the configured
/// limits, and give its terminal key focus. Returns the new session's id.
pub fn spawn_new_session(
    editor: &mut Editor,
    name: Option<String>,
) -> anyhow::Result<AgentSessionId> {
    // Copy the needed config out so the config guard is dropped before the
    // mutable borrow of `editor.agents`.
    let (binary_path, extra_args, scrollback_lines, max_sessions) = {
        let config = editor.config();
        let cc = &config.claude_code;
        (
            cc.binary_path.clone(),
            cc.extra_args.clone(),
            cc.scrollback_lines,
            cc.max_sessions,
        )
    };

    if editor.agents.len() >= max_sessions {
        anyhow::bail!("maximum number of agent sessions ({max_sessions}) reached");
    }

    let cwd = helix_core::find_workspace().0;
    let display_name = name.unwrap_or_else(|| format!("agent {}", editor.agents.len() + 1));

    let id = editor.agents.spawn_session(SpawnConfig {
        display_name,
        cwd,
        program: binary_path,
        args: extra_args,
        envs: Vec::new(),
        worktree: None,
        settings_path: None,
        claude_session_id: None,
        scrollback_lines,
    })?;

    // Focus the terminal immediately so the user can start typing.
    editor.agents.list_focused = false;
    Ok(id)
}
