//! Claude Code agent panel UI.
//!
//! A floating window listing managed `claude` sessions (left) with the focused
//! session's embedded terminal on the right. See [`panel::ClaudePanel`].

pub mod diff_view;
pub mod panel;

pub use panel::ClaudePanel;

use std::path::PathBuf;

use helix_view::agent::{AgentSessionId, SpawnConfig, WorktreeInfo};
use helix_view::Editor;

/// Stable compositor id for the panel layer.
pub const ID: &str = "claude-panel";

/// Spawn a new agent session rooted at the workspace, honouring the configured
/// limits, and give its terminal key focus. Returns the new session's id.
pub fn spawn_new_session(
    editor: &mut Editor,
    name: Option<String>,
) -> anyhow::Result<AgentSessionId> {
    let cwd = helix_core::find_workspace().0;
    spawn_session_in(editor, name, cwd, None, None)
}

/// Spawn an agent session rooted at `cwd` (e.g. a git worktree). `worktree`
/// records ownership for cleanup. When `resume` is `Some(session_id)` the agent
/// continues that Claude session (`--resume`) instead of starting a fresh one.
/// Wires Claude Code hooks for live status when possible, falling back to a
/// plain spawn if that setup fails.
pub fn spawn_session_in(
    editor: &mut Editor,
    name: Option<String>,
    cwd: PathBuf,
    worktree: Option<WorktreeInfo>,
    resume: Option<String>,
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

    let display_name = name.unwrap_or_else(|| format!("agent {}", editor.agents.len() + 1));

    // The session id we track: reuse the resumed id (so hooks still correlate),
    // otherwise mint a fresh uuid.
    let session_id = resume
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    // Best-effort hooks wiring: live status needs the socket + per-session
    // settings, but a failure here must not stop the agent from spawning.
    let (claude_session_id, settings_path, mut args) = match setup_session_hooks(editor) {
        Ok(settings) => {
            let id_flag = if resume.is_some() {
                "--resume"
            } else {
                "--session-id"
            };
            let args = vec![
                id_flag.to_string(),
                session_id.clone(),
                "--settings".to_string(),
                settings.to_string_lossy().into_owned(),
            ];
            (Some(session_id), Some(settings), args)
        }
        Err(err) => {
            log::warn!("Claude agent hooks disabled (status will not update): {err}");
            // Still honor resume so the conversation continues.
            let args = match &resume {
                Some(id) => vec!["--resume".to_string(), id.clone()],
                None => Vec::new(),
            };
            (resume.clone(), None, args)
        }
    };
    // Caller-supplied extra args come first, our control flags last.
    let mut full_args = extra_args;
    full_args.append(&mut args);

    let id = editor.agents.spawn_session(SpawnConfig {
        display_name,
        cwd,
        program: binary_path,
        args: full_args,
        envs: Vec::new(),
        worktree,
        settings_path,
        claude_session_id,
        scrollback_lines,
    })?;

    // Focus the terminal immediately so the user can start typing.
    editor.agents.list_focused = false;
    Ok(id)
}

/// Ensure the hook listener is running and generate this session's settings
/// file. Returns the settings path.
fn setup_session_hooks(editor: &mut Editor) -> anyhow::Result<PathBuf> {
    let socket = crate::agent::hooks::ensure_listener(editor)?;
    let settings = crate::agent::hooks::generate_settings(&socket)?;
    Ok(settings)
}
