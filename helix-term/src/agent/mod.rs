//! Editor-side orchestration for Claude Code agent sessions.
//!
//! The session *state* lives in [`helix_view::agent`]; this module holds the
//! parts that depend on `helix-term` (the job/callback machinery, the editor
//! binary itself): the Claude Code **hooks** bridge that drives each session's
//! status out-of-band.

pub mod hooks;
