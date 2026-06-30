//! Claude Code hooks bridge.
//!
//! We learn each agent session's status ("working / waiting / done") from
//! Claude Code's own hook events, with zero coupling to any stream-json
//! internals. The mechanism, end to end:
//!
//! 1. Each `claude` process is spawned with `--settings <generated file>` whose
//!    hooks invoke **this editor binary** as a tiny forwarder:
//!    `hx --agent-hook-emit <socket>`.
//! 2. That forwarder ([`forward_stdin_to_socket`]) reads the hook's JSON payload
//!    from stdin and writes it to an editor-owned Unix domain socket, then exits.
//! 3. A listener task ([`ensure_listener`]) accepts each connection, parses the
//!    payload, maps `session_id` → [`AgentSessionId`], and updates the session's
//!    status on the main loop via a `Callback::Editor`.
//!
//! Everything Unix-socket is `#[cfg(unix)]`; on other platforms the bridge is a
//! no-op (the panel still works, just without live status).

use std::path::{Path, PathBuf};

use helix_view::agent::AgentStatus;
use helix_view::notifications::{Notification, NotificationAction};
use helix_view::Editor;

use crate::compositor::Compositor;

/// One hook event, parsed leniently. Unknown events deserialize fine and map to
/// no status change, so an evolving hook schema degrades gracefully rather than
/// erroring. Fields are whatever Claude Code documents for hook stdin payloads.
#[derive(Debug, Default, serde::Deserialize)]
struct HookPayload {
    #[serde(default)]
    hook_event_name: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    /// Present on `Notification` events — the human-readable message.
    #[serde(default)]
    message: Option<String>,
    /// Present on tool hooks.
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<ToolInput>,
    /// Cumulative session cost, if the hook reports it (e.g. on `Stop`). Both
    /// spellings are accepted since the field name isn't firmly documented.
    #[serde(default)]
    total_cost_usd: Option<f64>,
    #[serde(default)]
    cost_usd: Option<f64>,
    #[serde(default)]
    usage: Option<Usage>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct ToolInput {
    #[serde(default)]
    file_path: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
}

/// Map a hook event name to the status it implies, or `None` for events that
/// should not change status (e.g. `SessionEnd`, handled by the exit watcher, or
/// any unrecognized event).
fn status_for(event: &str, message: Option<String>) -> Option<AgentStatus> {
    match event {
        "SessionStart" => Some(AgentStatus::Starting),
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" | "PreCompact" => {
            Some(AgentStatus::Working)
        }
        "Notification" => Some(AgentStatus::AwaitingAttention(
            message.unwrap_or_else(|| "needs attention".to_string()),
        )),
        "Stop" | "SubagentStop" => Some(AgentStatus::Done),
        _ => None,
    }
}

/// The forwarder mode (`hx --agent-hook-emit <socket>`): read the hook JSON from
/// stdin, ship it to the editor's socket, and exit. Always returns 0 — a hook
/// must never fail in a way that disrupts the `claude` run, so if the editor
/// isn't listening we silently succeed.
#[cfg(unix)]
pub fn forward_stdin_to_socket(socket: &Path) -> i32 {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        return 0;
    }
    if let Ok(mut stream) = UnixStream::connect(socket) {
        let _ = stream.write_all(&input);
        let _ = stream.flush();
    }
    0
}

#[cfg(not(unix))]
pub fn forward_stdin_to_socket(_socket: &Path) -> i32 {
    0
}

/// Ensure the hook listener is running and return the socket path to embed in a
/// session's generated settings. Lazily binds the socket and spawns the accept
/// loop on first use; subsequent calls return the existing path.
#[cfg(unix)]
pub fn ensure_listener(editor: &mut Editor) -> std::io::Result<PathBuf> {
    if let Some(path) = &editor.agents.hook_socket {
        return Ok(path.clone());
    }

    let path = std::env::temp_dir().join(format!("helix-agent-{}.sock", std::process::id()));
    // A stale socket from a previous run with the same pid would block bind.
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path)?;

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    tokio::spawn(handle_connection(stream));
                }
                // The listener is gone (socket removed on shutdown); stop.
                Err(_) => break,
            }
        }
    });

    editor.agents.hook_socket = Some(path.clone());
    Ok(path)
}

#[cfg(not(unix))]
pub fn ensure_listener(_editor: &mut Editor) -> std::io::Result<PathBuf> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "agent hooks require a Unix platform",
    ))
}

#[cfg(unix)]
async fn handle_connection(mut stream: tokio::net::UnixStream) {
    use tokio::io::AsyncReadExt;

    let mut buf = Vec::new();
    if stream.read_to_end(&mut buf).await.is_err() {
        return;
    }
    apply_payload(&buf).await;
}

/// Parse one payload and push a main-loop callback that updates the matching
/// session. Pure-ish: split out from the socket plumbing so it can be unit
/// tested. Returns the parsed payload for tests; the production path ignores it.
#[cfg(unix)]
async fn apply_payload(buf: &[u8]) {
    let payload: HookPayload = match serde_json::from_slice(buf) {
        Ok(p) => p,
        // Malformed or unrecognized shape: ignore rather than disrupt anything.
        Err(_) => return,
    };

    // EditorCompositor (not Editor-only) so we can tell whether the agent panel
    // is currently open when deciding to raise a toast.
    crate::job::dispatch(move |editor, compositor| {
        apply_to_editor(editor, compositor, payload);
    })
    .await;
    helix_event::request_redraw();
}

/// Correlate a payload to a session, mutate it, and raise a toast on a
/// status transition the user can't already see. Runs on the main loop.
fn apply_to_editor(editor: &mut Editor, compositor: &mut Compositor, payload: HookPayload) {
    // Prefer the session-id we passed via `--session-id`; fall back to cwd in
    // case the installed CLI doesn't echo our id back on hook payloads.
    let id = payload
        .session_id
        .as_deref()
        .and_then(|sid| editor.agents.id_for_claude_session(sid))
        .or_else(|| {
            payload
                .cwd
                .as_deref()
                .and_then(|cwd| editor.agents.id_for_cwd(Path::new(cwd)))
        });

    let Some(id) = id else { return };
    let (notify_on_attention, notify_on_done) = {
        let cc = &editor.config().claude_code;
        (cc.notify_on_attention, cc.notify_on_done)
    };
    let new_status = status_for(&payload.hook_event_name, payload.message.clone());

    // "Already there": the panel is open AND this session is focused AND the
    // terminal (not the list) has focus — i.e. the user is in this thread.
    let panel_open = compositor
        .find_id::<crate::ui::overlay::Overlay<crate::ui::claude::ClaudePanel>>(
            crate::ui::claude::ID,
        )
        .is_some();
    let user_there =
        panel_open && editor.agents.focused == Some(id) && !editor.agents.list_focused;

    // Mutate the session in a scope so the borrow ends before we push a toast.
    let mut toast = None;
    {
        let Some(session) = editor.agents.get_mut(id) else {
            return;
        };
        let name = session.display_name.clone();

        if let Some(status) = &new_status {
            let transitioned = session.status != *status;
            session.status = status.clone();

            if transitioned && !user_there {
                match status {
                    AgentStatus::AwaitingAttention(message) if notify_on_attention => {
                        toast = Some(
                            Notification::warning(format!("{name}: {message}"))
                                .with_title("agent blocked")
                                .sticky()
                                .with_action(NotificationAction::FocusAgent(id)),
                        );
                    }
                    AgentStatus::Done if notify_on_done => {
                        toast = Some(
                            Notification::info(format!("{name} finished"))
                                .with_title("agent done")
                                .with_action(NotificationAction::FocusAgent(id)),
                        );
                    }
                    _ => {}
                }
            }
        }
        session.last_activity = std::time::Instant::now();

        // Accounting from the Stop payload (best-effort; fields may be absent).
        if matches!(payload.hook_event_name.as_str(), "Stop" | "SubagentStop") {
            session.stats.turn_count = session.stats.turn_count.saturating_add(1);
        }
        if let Some(cost) = payload.total_cost_usd.or(payload.cost_usd) {
            session.stats.cost_usd = cost;
        }
        if let Some(usage) = &payload.usage {
            if let Some(input) = usage.input_tokens {
                session.stats.input_tokens = input;
            }
            if let Some(output) = usage.output_tokens {
                session.stats.output_tokens = output;
            }
        }

        // Record files the agent edited, for the diff view.
        if payload.hook_event_name == "PostToolUse"
            && matches!(
                payload.tool_name.as_deref(),
                Some("Edit") | Some("Write") | Some("MultiEdit")
            )
        {
            if let Some(file) = payload.tool_input.and_then(|t| t.file_path) {
                session.edited_files.insert(PathBuf::from(file));
            }
        }
    }

    if let Some(toast) = toast {
        editor.push_notification(toast);
    }
}

/// Generate a per-session `settings.json` wiring Claude Code's hooks to this
/// editor's forwarder, and return its path. Stored on the session and removed on
/// close.
pub fn generate_settings(socket: &Path) -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let command = format!(
        "{} --agent-hook-emit {}",
        sh_quote(&exe.to_string_lossy()),
        sh_quote(&socket.to_string_lossy()),
    );

    // Every event we care about forwards the same way; matcher "*" covers all
    // tools for the tool hooks.
    let entry = serde_json::json!([{ "hooks": [{ "type": "command", "command": command }] }]);
    let tool_entry =
        serde_json::json!([{ "matcher": "*", "hooks": [{ "type": "command", "command": command }] }]);
    let settings = serde_json::json!({
        "hooks": {
            "SessionStart": entry,
            "UserPromptSubmit": entry,
            "PreToolUse": tool_entry,
            "PostToolUse": tool_entry,
            "Notification": entry,
            "Stop": entry,
            "SubagentStop": entry,
            "SessionEnd": entry,
        }
    });

    let path = std::env::temp_dir().join(format!("helix-claude-settings-{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&path, serde_json::to_vec_pretty(&settings)?)?;
    Ok(path)
}

/// Quote a string for a POSIX shell by wrapping in single quotes (Claude runs
/// hook commands via the shell).
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> HookPayload {
        serde_json::from_str(s).expect("valid payload")
    }

    #[test]
    fn parses_session_start() {
        let p = parse(r#"{"hook_event_name":"SessionStart","session_id":"abc","cwd":"/x"}"#);
        assert_eq!(p.hook_event_name, "SessionStart");
        assert_eq!(p.session_id.as_deref(), Some("abc"));
        assert!(matches!(
            status_for(&p.hook_event_name, None),
            Some(AgentStatus::Starting)
        ));
    }

    #[test]
    fn parses_post_tool_use_with_file() {
        let p = parse(
            r#"{"hook_event_name":"PostToolUse","session_id":"s","tool_name":"Edit","tool_input":{"file_path":"/a/b.rs"}}"#,
        );
        assert_eq!(p.tool_name.as_deref(), Some("Edit"));
        assert_eq!(p.tool_input.unwrap().file_path.as_deref(), Some("/a/b.rs"));
        assert!(matches!(
            status_for(&p.hook_event_name, None),
            Some(AgentStatus::Working)
        ));
    }

    #[test]
    fn notification_carries_message() {
        let p = parse(r#"{"hook_event_name":"Notification","message":"Permission needed"}"#);
        match status_for(&p.hook_event_name, p.message.clone()) {
            Some(AgentStatus::AwaitingAttention(m)) => assert_eq!(m, "Permission needed"),
            other => panic!("expected attention, got {other:?}"),
        }
    }

    #[test]
    fn unknown_event_is_noop() {
        let p = parse(r#"{"hook_event_name":"SomethingNew","session_id":"s","extra":42}"#);
        assert!(status_for(&p.hook_event_name, None).is_none());
    }

    #[test]
    fn stop_is_done_and_missing_fields_default() {
        let p = parse(r#"{"hook_event_name":"Stop"}"#);
        assert!(p.session_id.is_none());
        assert!(matches!(
            status_for(&p.hook_event_name, None),
            Some(AgentStatus::Done)
        ));
    }

    #[test]
    fn stop_payload_carries_cost_and_usage() {
        let p = parse(
            r#"{"hook_event_name":"Stop","session_id":"s","total_cost_usd":0.42,"usage":{"input_tokens":1200,"output_tokens":340}}"#,
        );
        assert_eq!(p.total_cost_usd, Some(0.42));
        let usage = p.usage.expect("usage present");
        assert_eq!(usage.input_tokens, Some(1200));
        assert_eq!(usage.output_tokens, Some(340));
    }

    #[test]
    fn sh_quote_escapes_single_quotes() {
        assert_eq!(sh_quote("/a b/c"), "'/a b/c'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }
}
