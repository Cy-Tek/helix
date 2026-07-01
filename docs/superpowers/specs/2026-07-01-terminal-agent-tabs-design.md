# Design: Terminals & Agents as Editor Buffers (Tabs)

**Date:** 2026-07-01
**Status:** Approved (design) — pending implementation plan
**Author:** Josh Hannaford (with Claude)

## Context & Motivation

This Helix fork has a Claude **agent panel** and an embedded **`:terminal`**, both implemented as
floating **compositor overlays** (`helix-term/src/ui/claude/panel.rs`,
`helix-term/src/ui/terminal.rs`) over the emulator wrapper
`helix-view/src/terminal.rs` (`TerminalHandle`). Agent sessions live in an `AgentRegistry` at
`editor.agents`.

Today these panes can only be floating. We want the **option** to also open a terminal or agent as a
**full-screen editor buffer/tab**, so the user can navigate to it with normal buffer keys
(`gp`/`gn` = `goto_previous_buffer`/`goto_next_buffer`), use splits, and treat it like any other
buffer — **while keeping the existing floating-pane behavior unchanged**. The two views must be
windows onto the *same* live session, not separate instances.

Additionally, agent status events (blocked/finished) should be able to focus the specific session's
tab, via a properly decoupled event-distribution layer.

## Confirmed Decisions

1. **Modal input.** Over a terminal tab: **Normal mode = editor** (`gp`/`gn`, splits, `:` commands
   all work); **Insert mode = keystrokes forwarded to the PTY**. `Esc` → Normal; **`Shift-Esc` sends
   a real `ESC` (0x1b) to the terminal**.
2. **Per-session tabs, shared with floating.** A tab shows one agent's terminal (or one standalone
   `:terminal`) full-screen — the *same* live session as the registry/floating panel. `gp`/`gn`
   cycles tabs alongside file buffers.
3. **Close view only.** Closing a tab removes the view; the session keeps running in its registry
   (still in the floating panel, reopenable as a tab, still firing toasts).
4. **Event focus is user-driven.** A blocked/finished toast, when acted on (`space n`), focuses/opens
   that session's tab. No auto-focus/steal.
5. **Event bus reuses `helix_event`.** Formalize a typed `AgentStatusChanged` event on the existing
   bus; toast + future consumers subscribe. No parallel queue.

## Architecture

### 1. Data model & ownership

Terminals live in **persistent registries**; a tab is a throwaway **host `Document`** that maps to
one.

- `editor.agents` (existing `AgentRegistry`, keyed by `AgentSessionId`) — unchanged.
- **New** `editor.terminals: slotmap::SlotMap<TerminalId, TerminalHandle>` (with
  `new_key_type! { pub struct TerminalId; }`), so a `:terminal` outlives any single view. This
  **mirrors the sibling `AgentRegistry.sessions` registry**, which is already a
  `slotmap::SlotMap<AgentSessionId, AgentSession>` — same construction (`insert_with_key`,
  `get`/`get_mut`/`remove`) and same generational stale-key safety (a closed id never resolves to a
  different terminal). `slotmap` is already a workspace dependency. The floating `TerminalPane` is
  refactored to *reference* a registry handle instead of owning it, so one terminal can be shown
  floating **or** as a tab.
- **New** binding map:
  ```rust
  // on Editor
  terminal_docs: HashMap<DocumentId, TerminalRef>,
  enum TerminalRef { Agent(AgentSessionId), Standalone(TerminalId) }
  ```
- **Opening as a tab:** create a **read-only, path-less host `Document`** (`Document::default`,
  `readonly = true`), register via `new_document` → `DocumentId`, insert `doc_id → TerminalRef`, then
  `editor.switch(doc_id, Action::Replace)`.
- The host doc's Rope stays empty forever (input is intercepted before it can mutate it). It exists
  only to be a citizen of `editor.documents`, so `gp`/`gn`, splits, and the bufferline work with **no
  changes** to those systems.
- Resolve a ref to `&TerminalHandle` via a helper `Editor::resolve_terminal(&TerminalRef)`:
  `Agent(id)` → `agents.get(id)?.terminal`; `Standalone(tid)` → `terminals.get(tid)`.

Rationale for registries (not doc-owned handles): decision #3 requires the handle to survive the
view; the doc never owns it.

### 2. Rendering & cursor

- **Enabling change:** `TerminalHandle::resize` becomes `&self` (the `size` field moves behind a
  `Cell`; `MasterPty::resize` and `Term::resize` are already interior-mutable). This lets rendering
  resize the emulator from the immutable `&Editor` render path, and simplifies the floating pane
  (no `focused_mut()` just to resize).
- **Render branch** in `EditorView::render_view` (`helix-term/src/ui/editor.rs`, immediately before
  the `render_document` call ~line 259):
  ```rust
  if let Some(term_ref) = editor.terminal_docs.get(&view.doc) {
      if let Some(handle) = editor.resolve_terminal(term_ref) {
          handle.resize(inner.height, inner.width);
          let cursor = crate::ui::terminal::render(handle, inner, surface); // reuse existing blitter
          self.stash_terminal_cursor(view.id, cursor);
      }
      return; // skip gutters/text/diagnostics/rulers for this view
  }
  ```
- No floating border/title (it's a full pane). The **statusline** still renders below; the
  **bufferline** shows the tab up top. Splits work because each `View` gets its own `inner`.
- **Cursor:** `EditorView::cursor` special-cases a focused terminal-doc view: in **Insert** mode
  return the terminal cursor (block/bar); otherwise hidden. (Cursor only sits in the terminal when
  you're driving it.)
- **Edge cases:** zero-area / exited / unresolvable refs render as a blank pane; an unresolvable ref
  falls through to the empty read-only host doc and the stale mapping is cleaned up.

### 3. Modal input routing

No new mode enum — rides Helix's Normal/Insert. Early check in `EditorView`'s key path: *is the
focused view's doc in `terminal_docs`?*

- **Normal mode:** keys flow through the normal keymap unchanged (`gp`/`gn`, `Ctrl-w`, `:`…). `i`/`a`
  enter Insert. The `readonly` host doc makes any editing command inert (no buffer corruption).
- **Insert mode:** keys are intercepted *before* the editor's insert path and forwarded via the
  existing `terminal::encode_key` + paste handling:

  | Key | Action |
  |---|---|
  | `Esc` (no mods) | Leave Insert → **Normal** (not sent to terminal) |
  | `Shift-Esc` | Forward real **`ESC` (0x1b)** to the PTY |
  | anything else | `encode_key` → PTY |

  `Shift-Esc` relies on enhanced keyboard reporting (`Esc`+`SHIFT`); where unavailable, plain `Esc`
  still returns to Normal and claude's Ctrl interrupts still work.

- Interception consumes keys in Insert, so the readonly host doc is never mutated. Since `gp`/`gn`
  only live in Normal, leaving a terminal is always a deliberate `Esc`-then-navigate.

### 4. Opening / closing / display

- **Open (proposed defaults; keybindings easily tweaked):** floating panel key `t` = open focused
  session as a tab; typable commands `:agent-tab` and `:terminal-tab [cmd]`. Existing floating
  commands unchanged.
- **Close (view only):** closing the view drops the host `Document` + `terminal_docs` entry; the
  `TerminalHandle` stays in its registry. Cleanup hooks into the existing `DocumentDidClose` event.
- **Naming:** `Editor::doc_display_name(doc_id)` checks `terminal_docs` first → `[agent 1]`
  (from `AgentSession.display_name`) / `[term: zsh]`, else the normal filename/`[scratch]` logic.
  Used by both `render_bufferline` and the statusline.

### 5. Agent event bus (on `helix_event`)

- Add `AgentStatusChanged { session_id, status }` via the `events!` macro in **`helix-view`** (both
  crates see it). Register it at startup alongside existing events.
- `helix-term/src/agent/hooks.rs::apply_to_editor` shrinks to its authoritative job — correlate the
  claude hook payload to a session, update `status`/stats/edited-files — then
  `helix_event::dispatch(AgentStatusChanged{…})`.
- The **toast decision moves into a subscriber** (suppress-when-watching, `notify_on_attention` /
  `notify_on_done`, push the toast with a `FocusAgent` action). Because `helix_event` hooks can't
  touch the editor directly, editor-mutating subscribers use `job::dispatch` — the same async→
  main-loop bridge the hooks already use (one-frame defer, fine for async toasts). Status-correlation
  and UI reaction are thus decoupled; future systems just `register_hook`.
- **Tab-focus on toast activation:** retarget the existing `notification_action` command (`space n`)
  so `NotificationAction::FocusAgent(id)` **focuses that session's tab** (switch to it if a host doc
  exists, else create one) instead of opening the floating panel. That command already holds
  editor+compositor access → no auto-steal (decision #4).

### 6. Mouse in tabs

- **Wheel:** in `EditorView`'s mouse handling, if the pointer is over a terminal-doc view, forward
  the wheel to that handle via the existing `TerminalHandle::wheel`.
- **Select/copy:** relax the compositor's screen-grid selection gate so it engages when the **focused
  view is a terminal-doc** (not only when an overlay is open), giving drag-select-and-copy in tabs
  for free by reusing the existing selection code.

## Key Files

- `helix-view/src/editor.rs` — `terminals` registry, `terminal_docs` map, `TerminalRef`,
  `resolve_terminal`, `doc_display_name`, open/close helpers.
- `helix-view/src/terminal.rs` — `resize(&self)` (interior-mutable `size`).
- `helix-view/src/events.rs` (or `agent/mod.rs`) — `AgentStatusChanged` event.
- `helix-view/src/agent/mod.rs` — standalone-terminal registry integration if needed.
- `helix-term/src/ui/editor.rs` — render branch, cursor special-case, modal input routing, wheel,
  bufferline naming.
- `helix-term/src/ui/terminal.rs` / `claude/panel.rs` — refactor to reference the terminal registry;
  panel `t` binding.
- `helix-term/src/agent/hooks.rs` — dispatch `AgentStatusChanged`; move toast logic to a subscriber.
- `helix-term/src/commands.rs` — `:agent-tab` / `:terminal-tab`; retarget `notification_action`.
- `helix-term/src/compositor.rs` — selection gate also engages for focused terminal-doc views.

## Testing / Verification

- **Unit:** `resolve_terminal` mapping; `doc_display_name` for agent/terminal/file/scratch;
  `encode_key`/`Shift-Esc` byte output (extend existing `ui::terminal::tests`).
- **Manual (release binary):**
  1. Open an agent as a tab (`t` / `:agent-tab`); confirm full-screen render, statusline + bufferline
     show `[agent N]`.
  2. Normal mode: `gp`/`gn` cycle between the tab and file buffers; `Ctrl-w` split shows the terminal
     in both panes live.
  3. `i` → type into claude; `Esc` → Normal; `Shift-Esc` → claude receives ESC.
  4. Wheel scrolls the tab; drag-select + copy works; the buffer behind is unaffected.
  5. Close the tab (`:bc`) → session still in the floating panel; reopen as a tab shows the same live
     session.
  6. Trigger a blocked/finished toast; `space n` focuses/opens that session's tab.
  7. Floating panel still works exactly as before.

## Out of Scope / Future

- Persisting terminal tabs across editor restarts.
- Auto-focus modes (idle/always) — deliberately deferred (decision #4).
- Non-terminal special buffers (diffs/previews) — the data model leaves room but they're not built.
