//! A generic, stackable toast-notification queue.
//!
//! This is editor-wide and event-agnostic: any subsystem can push a
//! [`Notification`] (info/warning/error), optionally with a timeout and an
//! [`NotificationAction`] that a single universal key can act on. The data lives
//! here on plain types (no `helix-term` dependency); rendering, the auto-dismiss
//! timer, and the action key live in `helix-term`.

use std::borrow::Cow;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::agent::AgentSessionId;
use crate::editor::Severity;

/// Default lifetime for a non-sticky toast.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// What a toast's action key does. Extensible: new event types add variants.
/// `RunCommand` is a generic escape hatch (run a static command by name).
#[derive(Debug, Clone)]
pub enum NotificationAction {
    /// Open the agent panel (if needed) and focus this session.
    FocusAgent(AgentSessionId),
    /// Run a named command (future-proofing for non-agent toasts).
    RunCommand(String),
}

/// A single toast.
#[derive(Debug, Clone)]
pub struct Notification {
    pub id: u64,
    pub severity: Severity,
    pub title: Option<Cow<'static, str>>,
    pub body: Cow<'static, str>,
    pub created_at: Instant,
    /// `None` means sticky (never auto-dismisses).
    pub timeout: Option<Duration>,
    pub action: Option<NotificationAction>,
}

impl Notification {
    fn base(severity: Severity, body: impl Into<Cow<'static, str>>) -> Self {
        Self {
            id: 0,
            severity,
            title: None,
            body: body.into(),
            created_at: Instant::now(),
            timeout: Some(DEFAULT_TIMEOUT),
            action: None,
        }
    }

    pub fn info(body: impl Into<Cow<'static, str>>) -> Self {
        Self::base(Severity::Info, body)
    }

    pub fn warning(body: impl Into<Cow<'static, str>>) -> Self {
        Self::base(Severity::Warning, body)
    }

    pub fn error(body: impl Into<Cow<'static, str>>) -> Self {
        Self::base(Severity::Error, body)
    }

    pub fn with_title(mut self, title: impl Into<Cow<'static, str>>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Make this toast persist until it is acted on or dismissed.
    pub fn sticky(mut self) -> Self {
        self.timeout = None;
        self
    }

    pub fn with_action(mut self, action: NotificationAction) -> Self {
        self.action = Some(action);
        self
    }

    /// Whether this toast has expired relative to `now`.
    fn is_expired(&self, now: Instant) -> bool {
        match self.timeout {
            Some(timeout) => now.duration_since(self.created_at) >= timeout,
            None => false,
        }
    }

    /// The absolute instant this toast expires, if it is timed.
    fn expiry(&self) -> Option<Instant> {
        self.timeout.map(|t| self.created_at + t)
    }
}

/// The editor-wide toast queue. Oldest at the front, newest at the back.
#[derive(Debug, Default)]
pub struct Notifications {
    items: VecDeque<Notification>,
    next_id: u64,
    /// The expiry instant a wake timer was last scheduled for, so the renderer
    /// only spawns a new timer when the earliest expiry actually changes.
    scheduled_wake: Option<Instant>,
}

impl Notifications {
    /// Push a toast, assigning it a fresh id and stamping its creation time.
    /// Returns the id (usable with [`dismiss`](Self::dismiss)).
    pub fn push(&mut self, mut notification: Notification) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        notification.id = id;
        notification.created_at = Instant::now();
        self.items.push_back(notification);
        id
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Toasts oldest-first. Double-ended so callers can render newest-first.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Notification> + ExactSizeIterator {
        self.items.iter()
    }

    pub fn dismiss(&mut self, id: u64) {
        self.items.retain(|n| n.id != id);
    }

    /// Dismiss the newest toast (for the "dismiss" key).
    pub fn dismiss_newest(&mut self) {
        self.items.pop_back();
    }

    /// Drop every toast that has timed out. Sticky toasts are kept.
    pub fn retain_unexpired(&mut self, now: Instant) {
        self.items.retain(|n| !n.is_expired(now));
    }

    /// Remove and return the action of the newest toast that has one. Toasts
    /// without an action are skipped (and left in place).
    pub fn take_newest_action(&mut self) -> Option<NotificationAction> {
        let idx = self.items.iter().rposition(|n| n.action.is_some())?;
        self.items.remove(idx).and_then(|n| n.action)
    }

    /// The soonest expiry instant across all timed toasts.
    pub fn earliest_expiry(&self) -> Option<Instant> {
        self.items.iter().filter_map(Notification::expiry).min()
    }

    /// If the earliest expiry has changed since the last scheduled wake, record
    /// the new value and return it so the caller can arm a timer. Returns `None`
    /// when no new timer is needed (unchanged, or no timed toasts remain).
    pub fn take_pending_wake(&mut self) -> Option<Instant> {
        let next = self.earliest_expiry();
        if next != self.scheduled_wake {
            self.scheduled_wake = next;
            next
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_assigns_unique_ids_and_orders_oldest_first() {
        let mut n = Notifications::default();
        let a = n.push(Notification::info("a"));
        let b = n.push(Notification::info("b"));
        assert_ne!(a, b);
        let bodies: Vec<_> = n.iter().map(|t| t.body.to_string()).collect();
        assert_eq!(bodies, vec!["a", "b"]);
    }

    #[test]
    fn retain_unexpired_drops_only_timed_out_and_keeps_sticky() {
        let mut n = Notifications::default();
        n.push(Notification::info("fades").with_timeout(Duration::from_millis(10)));
        n.push(Notification::warning("stays").sticky());
        // Far enough in the future that the timed toast has expired.
        let later = Instant::now() + Duration::from_secs(1);
        n.retain_unexpired(later);
        let bodies: Vec<_> = n.iter().map(|t| t.body.to_string()).collect();
        assert_eq!(bodies, vec!["stays"]);
    }

    #[test]
    fn take_newest_action_returns_newest_actionable_and_skips_info() {
        let mut n = Notifications::default();
        n.push(Notification::warning("blocked-A").sticky().with_action(NotificationAction::RunCommand("a".into())));
        n.push(Notification::info("finished")); // no action
        n.push(Notification::warning("blocked-B").sticky().with_action(NotificationAction::RunCommand("b".into())));

        match n.take_newest_action() {
            Some(NotificationAction::RunCommand(c)) => assert_eq!(c, "b"),
            other => panic!("expected newest actionable (b), got {other:?}"),
        }
        // The info toast and blocked-A remain; next action is blocked-A.
        assert_eq!(n.len(), 2);
        match n.take_newest_action() {
            Some(NotificationAction::RunCommand(c)) => assert_eq!(c, "a"),
            other => panic!("expected blocked-A, got {other:?}"),
        }
        assert!(n.take_newest_action().is_none());
    }

    #[test]
    fn earliest_expiry_picks_soonest_timed_toast() {
        let mut n = Notifications::default();
        n.push(Notification::info("slow").with_timeout(Duration::from_secs(60)));
        n.push(Notification::warning("sticky").sticky());
        n.push(Notification::info("fast").with_timeout(Duration::from_secs(1)));
        let earliest = n.earliest_expiry().expect("a timed toast exists");
        // "fast" (≈ now + 1s) must be sooner than "slow" (≈ now + 60s).
        assert!(earliest < Instant::now() + Duration::from_secs(30));
    }

    #[test]
    fn pending_wake_dedupes_until_expiry_changes() {
        let mut n = Notifications::default();
        n.push(Notification::info("a").with_timeout(Duration::from_secs(5)));
        let first = n.take_pending_wake();
        assert!(first.is_some(), "first timed toast arms a wake");
        assert!(n.take_pending_wake().is_none(), "unchanged earliest expiry: no new wake");
    }
}
