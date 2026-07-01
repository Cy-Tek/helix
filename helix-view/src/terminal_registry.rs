//! Managed standalone terminals, mirroring the agent registry.
//!
//! Each [`TerminalSession`] owns a live embedded terminal ([`TerminalHandle`])
//! running an arbitrary command (a shell by default). The [`TerminalRegistry`]
//! lives on the [`Editor`](crate::Editor) as plain data, mutated only on the
//! main loop, so it needs no locks of its own — the same ownership model as
//! [`AgentRegistry`](crate::agent::AgentRegistry).

use std::path::PathBuf;
use std::time::Instant;

use crate::terminal::TerminalHandle;

slotmap::new_key_type! {
    /// Editor-local handle to a standalone (`:terminal` / terminals-panel)
    /// embedded terminal, stable across removals — mirrors `AgentSessionId`.
    pub struct TerminalId;
}

/// Process lifecycle of a managed terminal, surfaced as a glyph in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalStatus {
    /// Registered but the child process has not been spawned yet. Reserved for
    /// future queued terminals; terminals spawned today start `InProgress`.
    NotStarted,
    /// The child process is running.
    InProgress,
    /// The child process exited with status 0.
    Succeeded,
    /// The child process exited with a non-zero status.
    Failed,
}

impl TerminalStatus {
    /// Whether this is a terminal (settled) state that no longer changes.
    pub fn is_finished(&self) -> bool {
        matches!(self, TerminalStatus::Succeeded | TerminalStatus::Failed)
    }
}

/// A single managed terminal: its emulator handle, what it runs, and its status.
pub struct TerminalSession {
    pub id: TerminalId,
    /// Human label shown in the list (defaults to the command).
    pub name: String,
    /// The command line this terminal runs (empty for a bare shell).
    pub command: String,
    pub cwd: PathBuf,
    pub status: TerminalStatus,
    pub terminal: TerminalHandle,
    pub last_activity: Instant,
}

impl TerminalSession {
    /// Refresh `status` from the child's exit state. Once the process has exited,
    /// the status settles to `Succeeded`/`Failed` and never changes again.
    pub fn refresh_status(&mut self) {
        if self.status.is_finished() {
            return;
        }
        match self.terminal.exit_status() {
            Some(0) => self.status = TerminalStatus::Succeeded,
            Some(_) => self.status = TerminalStatus::Failed,
            None => {
                if self.status == TerminalStatus::NotStarted {
                    self.status = TerminalStatus::InProgress;
                }
            }
        }
    }
}

/// All managed terminals, plus which one is focused and where keystrokes go.
/// Deliberately parallel to [`AgentRegistry`](crate::agent::AgentRegistry).
#[derive(Default)]
pub struct TerminalRegistry {
    sessions: slotmap::SlotMap<TerminalId, TerminalSession>,
    /// Insertion order, for a stable list rendering.
    order: Vec<TerminalId>,
    pub focused: Option<TerminalId>,
    /// When `true`, keystrokes drive the terminal list; when `false`, they are
    /// forwarded to the focused terminal.
    pub list_focused: bool,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self {
            list_focused: true,
            ..Default::default()
        }
    }

    /// Register an already-spawned terminal and focus it. Returns its id.
    pub fn register(
        &mut self,
        name: String,
        command: String,
        cwd: PathBuf,
        terminal: TerminalHandle,
    ) -> TerminalId {
        let id = self.sessions.insert_with_key(|id| TerminalSession {
            id,
            name,
            command,
            cwd,
            status: TerminalStatus::InProgress,
            terminal,
            last_activity: Instant::now(),
        });
        self.order.push(id);
        self.focused = Some(id);
        id
    }

    pub fn get(&self, id: TerminalId) -> Option<&TerminalSession> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: TerminalId) -> Option<&mut TerminalSession> {
        self.sessions.get_mut(id)
    }

    pub fn focused(&self) -> Option<&TerminalSession> {
        self.focused.and_then(|id| self.sessions.get(id))
    }

    pub fn focused_mut(&mut self) -> Option<&mut TerminalSession> {
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
    pub fn iter(&self) -> impl Iterator<Item = &TerminalSession> {
        self.order.iter().filter_map(move |id| self.sessions.get(*id))
    }

    /// Recompute every session's status from its child's exit state. Cheap
    /// (a non-blocking `try_wait` per terminal); called each render.
    pub fn refresh_statuses(&mut self) {
        for session in self.sessions.values_mut() {
            session.refresh_status();
        }
    }

    /// Whether any terminal is still running (for statusline / ticker gating).
    pub fn any_running(&self) -> bool {
        self.sessions
            .values()
            .any(|s| !s.status.is_finished())
    }

    /// Move focus to the next terminal in list order (wrapping).
    pub fn focus_next(&mut self) -> Option<TerminalId> {
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

    /// Move focus to the previous terminal in list order (wrapping).
    pub fn focus_prev(&mut self) -> Option<TerminalId> {
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

    /// Kill and remove a terminal, returning it. Focus moves to the first
    /// remaining terminal.
    pub fn remove(&mut self, id: TerminalId) -> Option<TerminalSession> {
        let session = self.sessions.remove(id)?;
        session.terminal.kill();
        self.order.retain(|other| *other != id);
        if self.focused == Some(id) {
            self.focused = self.order.first().copied();
        }
        Some(session)
    }

    /// Kill every child process. Called on editor shutdown.
    pub fn shutdown_all(&mut self) {
        for session in self.sessions.values() {
            session.terminal.kill();
        }
    }
}
