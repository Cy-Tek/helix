//! Claude Code agent sessions managed from inside the editor.
//!
//! Each [`AgentSession`] owns a live embedded terminal ([`terminal::TerminalHandle`])
//! running an interactive `claude` process. The [`AgentRegistry`] lives on the
//! [`Editor`](crate::Editor) as plain data; it is mutated only on the main loop
//! (directly, or via job callbacks from background tasks), so it needs no locks
//! of its own. The terminal *grid* inside each handle is separately shared with
//! its reader thread behind a fair mutex — see [`terminal`].

use std::path::PathBuf;
use std::time::Instant;

// The terminal emulator is a general-purpose facility (also used by the
// standalone `:terminal`); it lives at `helix_view::terminal`. Re-exported here
// for the agent API's convenience.
pub use crate::terminal::{TerminalCell, TerminalHandle, TerminalSize, TerminalSnapshot};

slotmap::new_key_type! {
    /// Editor-local handle to an agent session, stable across removals — the
    /// same role `LanguageServerId` plays for language servers.
    pub struct AgentSessionId;
}

/// Coarse lifecycle state, surfaced as a glyph in the session list. Detailed
/// transitions (`Working`/`AwaitingAttention`/`Done`) are driven by Claude Code
/// hooks in a later phase; until then sessions are `Starting` then `Working`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentStatus {
    /// Spawned, process not yet confirmed running.
    Starting,
    /// Actively working (default once spawned, refined by hooks later).
    Working,
    /// Waiting on the user — a permission prompt, a question, or idle input.
    /// The string is the human-readable notification message, if any.
    AwaitingAttention(String),
    /// The most recent turn finished.
    Done,
    /// The child process exited with the given status code (if known).
    Exited(Option<i32>),
}

impl AgentStatus {
    /// Whether this session needs the user's attention.
    pub fn is_awaiting(&self) -> bool {
        matches!(self, AgentStatus::AwaitingAttention(_))
    }
}

/// Which view the session's right pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RightPane {
    /// The live `claude` terminal (default).
    Terminal,
    /// A diff of the edits the agent has made so far.
    Edits,
}

impl Default for RightPane {
    fn default() -> Self {
        RightPane::Terminal
    }
}

/// Records that a session owns a git worktree we created, so cleanup can offer
/// to remove it.
#[derive(Debug, Clone)]
pub struct WorktreeInfo {
    pub path: PathBuf,
    pub branch: String,
}

/// Cost/usage accounting, populated from `Stop` hook payloads in a later phase.
#[derive(Debug, Clone, Default)]
pub struct AgentStats {
    pub turn_count: u32,
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// A single managed agent: its terminal, its place on disk, and its status.
pub struct AgentSession {
    pub id: AgentSessionId,
    pub display_name: String,
    /// The UUID we pass via `--session-id`; hooks report the same value,
    /// giving direct correlation. `None` until assigned.
    pub claude_session_id: Option<String>,
    pub cwd: PathBuf,
    /// `Some` when we created a worktree for this session (eligible for cleanup).
    pub worktree: Option<WorktreeInfo>,
    pub status: AgentStatus,
    pub right_pane: RightPane,
    pub terminal: TerminalHandle,
    pub stats: AgentStats,
    pub last_activity: Instant,
    /// Generated per-session settings file, removed on close. `None` until the
    /// hooks phase wires it up.
    pub settings_path: Option<PathBuf>,
}

/// Parameters for spawning a new agent session.
pub struct SpawnConfig {
    pub display_name: String,
    pub cwd: PathBuf,
    pub program: String,
    pub args: Vec<String>,
    pub envs: Vec<(String, String)>,
    pub worktree: Option<WorktreeInfo>,
    pub settings_path: Option<PathBuf>,
    pub claude_session_id: Option<String>,
    pub scrollback_lines: usize,
}

/// All agent sessions, plus which one is focused and where keystrokes go.
#[derive(Default)]
pub struct AgentRegistry {
    sessions: slotmap::SlotMap<AgentSessionId, AgentSession>,
    /// Insertion order, for a stable list rendering.
    order: Vec<AgentSessionId>,
    pub focused: Option<AgentSessionId>,
    /// When `true`, keystrokes drive the session list; when `false`, they are
    /// forwarded to the focused terminal.
    pub list_focused: bool,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self {
            list_focused: true,
            ..Default::default()
        }
    }

    /// Spawn a new session's terminal and register it. The terminal starts at a
    /// default size; the first render resizes it to the real pane.
    pub fn spawn_session(&mut self, config: SpawnConfig) -> anyhow::Result<AgentSessionId> {
        let terminal = TerminalHandle::spawn(
            &config.program,
            &config.args,
            &config.envs,
            &config.cwd,
            24,
            80,
            config.scrollback_lines,
        )?;

        let id = self.sessions.insert_with_key(|id| AgentSession {
            id,
            display_name: config.display_name,
            claude_session_id: config.claude_session_id,
            cwd: config.cwd,
            worktree: config.worktree,
            status: AgentStatus::Starting,
            right_pane: RightPane::default(),
            terminal,
            stats: AgentStats::default(),
            last_activity: Instant::now(),
            settings_path: config.settings_path,
        });
        self.order.push(id);
        self.focused = Some(id);
        Ok(id)
    }

    pub fn get(&self, id: AgentSessionId) -> Option<&AgentSession> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: AgentSessionId) -> Option<&mut AgentSession> {
        self.sessions.get_mut(id)
    }

    pub fn focused(&self) -> Option<&AgentSession> {
        self.focused.and_then(|id| self.sessions.get(id))
    }

    pub fn focused_mut(&mut self) -> Option<&mut AgentSession> {
        match self.focused {
            Some(id) => self.sessions.get_mut(id),
            None => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// Sessions in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &AgentSession> {
        self.order.iter().filter_map(move |id| self.sessions.get(*id))
    }

    /// Find a session by its claude session id (the UUID passed via
    /// `--session-id`), for correlating hook events.
    pub fn id_for_claude_session(&self, claude_session_id: &str) -> Option<AgentSessionId> {
        self.order.iter().copied().find(|id| {
            self.sessions
                .get(*id)
                .and_then(|s| s.claude_session_id.as_deref())
                == Some(claude_session_id)
        })
    }

    /// Whether any session needs attention (for statusline / notifications).
    pub fn any_awaiting(&self) -> bool {
        self.sessions.values().any(|s| s.status.is_awaiting())
    }

    /// Move focus to the next session in list order (wrapping). Returns the
    /// newly focused id.
    pub fn focus_next(&mut self) -> Option<AgentSessionId> {
        if self.order.is_empty() {
            return None;
        }
        let next = match self.focused {
            Some(cur) => {
                let pos = self.order.iter().position(|id| *id == cur).unwrap_or(0);
                self.order[(pos + 1) % self.order.len()]
            }
            None => self.order[0],
        };
        self.focused = Some(next);
        Some(next)
    }

    /// Move focus to the previous session in list order (wrapping).
    pub fn focus_prev(&mut self) -> Option<AgentSessionId> {
        if self.order.is_empty() {
            return None;
        }
        let prev = match self.focused {
            Some(cur) => {
                let pos = self.order.iter().position(|id| *id == cur).unwrap_or(0);
                self.order[(pos + self.order.len() - 1) % self.order.len()]
            }
            None => self.order[0],
        };
        self.focused = Some(prev);
        Some(prev)
    }

    /// Focus the next session awaiting attention, if any.
    pub fn focus_next_awaiting(&mut self) -> Option<AgentSessionId> {
        let target = self
            .order
            .iter()
            .copied()
            .find(|id| self.sessions.get(*id).is_some_and(|s| s.status.is_awaiting()));
        if let Some(id) = target {
            self.focused = Some(id);
        }
        target
    }

    /// Kill and remove a session, returning it so the caller can clean up an
    /// owned worktree / settings file.
    pub fn remove(&mut self, id: AgentSessionId) -> Option<AgentSession> {
        let session = self.sessions.remove(id)?;
        session.terminal.kill();
        self.order.retain(|other| *other != id);
        if self.focused == Some(id) {
            self.focused = self.order.first().copied();
        }
        Some(session)
    }

    /// Kill every child process. Called on editor shutdown. Worktrees are not
    /// auto-removed.
    pub fn shutdown_all(&mut self) {
        for session in self.sessions.values() {
            session.terminal.kill();
        }
    }
}
