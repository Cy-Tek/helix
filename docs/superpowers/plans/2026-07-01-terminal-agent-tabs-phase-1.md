# Terminals & Agents as Buffers — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a Claude agent or a standalone `:terminal` be opened as a full-screen, navigable editor buffer/tab (in addition to the existing floating panes), driven modally (Normal = editor, Insert = terminal).

**Architecture:** Terminals live in persistent registries (`editor.agents` already; add `editor.terminals`). A tab is a read-only, empty **host `Document`** mapped (`editor.terminal_docs: HashMap<DocumentId, TerminalRef>`) to a live `TerminalHandle`. One render branch in `EditorView::render_view` blits the terminal grid instead of text; one intercept in the `Event::Key` handler forwards Insert-mode keys to the PTY. Buffer navigation, splits, and the bufferline come for free because the host doc is a real citizen of `editor.documents`.

**Tech Stack:** Rust, Helix editor internals (`helix-view`, `helix-term`), `alacritty_terminal`, `slotmap`.

**Spec:** `docs/superpowers/specs/2026-07-01-terminal-agent-tabs-design.md`

**Scope note (Phase 1 vs 2):** Phase 1 delivers tabs end-to-end. The toast "jump to agent" action keeps its **current** behavior (opens the floating panel) — retargeting it to focus a tab, plus the `AgentStatusChanged` event bus, is **Phase 2** and is intentionally excluded here.

---

## Execution Approach (chosen)

**Use `superpowers:subagent-driven-development`** to implement this plan — this is the recommended and agreed approach.

- Work on a dedicated branch (not `master`), e.g. `feat/terminal-agent-tabs`.
- **One fresh subagent per task** (Tasks 1–11, in order). Give each subagent only its task section plus the spec path; it should not carry prior tasks' context.
- **Two-stage review between tasks:** after each subagent reports, (1) verify the task's build/test step actually passed by running it, then (2) review the diff for correctness and adherence to the task before starting the next task. Do not batch multiple tasks into one subagent.
- Tasks are ordered by dependency: T1 (resize) → T2–T5 (editor data model + lifecycle) → T6–T9 (rendering, cursor, input, mouse) → T10 (commands/binding/bufferline) → T11 (manual verification). Do not reorder.
- Each task ends with its own commit (messages are provided per task). Push the branch and open a PR after T11 passes.
- The four "confirm exact names against the code" flags (see Self-Review Notes) must be resolved by reading the code during the relevant task — subagents must not invent signatures.

---

## File Structure

| File | Responsibility (Phase 1 changes) |
|---|---|
| `helix-view/src/terminal.rs` | `resize(&self)` via interior-mutable size; unchanged emulator API otherwise. |
| `helix-view/src/editor.rs` | New `terminals` registry, `TerminalId`, `terminal_docs` map, `TerminalRef`, and helpers: `resolve_terminal`, `is_terminal_doc`, `focused_terminal`, `open_terminal_tab`, `open_agent_tab`, `doc_display_name`; cleanup on `DocumentDidClose`. |
| `helix-view/src/terminal_registry.rs` *(new)* | `TerminalId` key type + thin `TerminalRegistry` wrapper (optional; may inline in editor.rs). |
| `helix-term/src/ui/editor.rs` | Render branch (blit grid), cursor special-case, Insert-mode key intercept, wheel forwarding, bufferline naming. |
| `helix-term/src/ui/terminal.rs` | (already has `render`, `encode_key`, `wheel`) — no structural change; reused. |
| `helix-term/src/ui/claude/panel.rs` | `t` key = open focused agent as a tab. |
| `helix-term/src/commands/typed.rs` | `:agent-tab` and `:terminal-tab` typable commands. |
| `helix-term/src/compositor.rs` | Selection gate also engages when the focused view is a terminal-doc. |

---

## Task 1: `TerminalHandle::resize` becomes `&self`

Rendering happens on an immutable `&Editor`, so the emulator must be resizable without `&mut`. `MasterPty::resize` and `Term::resize` are already interior-mutable; only the `size` field needs `Cell`.

**Files:**
- Modify: `helix-view/src/terminal.rs`

- [ ] **Step 1: Make `size` interior-mutable**

In the `TerminalHandle` struct (currently `size: TerminalSize,`) change to:

```rust
use std::cell::Cell;
// ...
    size: Cell<TerminalSize>,
```

Update construction in `spawn` (currently `size,`) to `size: Cell::new(size),`.

- [ ] **Step 2: Update `size()` and `resize()`**

```rust
    pub fn size(&self) -> TerminalSize {
        self.size.get()
    }

    /// Resize both the PTY and the emulator. No-op when unchanged.
    pub fn resize(&self, rows: u16, cols: u16) {
        let rows = rows.max(1);
        let cols = cols.max(1);
        let new = TerminalSize {
            columns: cols as usize,
            screen_lines: rows as usize,
        };
        if new == self.size.get() {
            return;
        }
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        self.term.lock().resize(new);
        self.size.set(new);
    }
```

- [ ] **Step 3: Fix existing callers that assumed `&mut`**

`helix-term/src/ui/terminal.rs` (`TerminalPane::render`) and `helix-term/src/ui/claude/panel.rs` (`ClaudePanel::render`) call `terminal.resize(...)` / `session.terminal.resize(...)`. These now work through `&` — in `panel.rs`, the `RightPane::Terminal` arm can use `ctx.editor.agents.focused()` (immutable) instead of `focused_mut()` for the resize. Adjust that one call to avoid an unnecessary `&mut` borrow:

```rust
            RightPane::Terminal => {
                self.term_grid = content_area;
                if let Some(session) = ctx.editor.agents.focused() {
                    session.terminal.resize(content_area.height, content_area.width);
                }
                self.cursor = ctx
                    .editor
                    .agents
                    .focused()
                    .and_then(|session| terminal::render(&session.terminal, content_area, surface));
```

- [ ] **Step 4: Build**

Run: `cargo build -p helix-view -p helix-term`
Expected: compiles clean (no `cannot borrow ... as mutable` errors around `resize`).

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/terminal.rs helix-term/src/ui/terminal.rs helix-term/src/ui/claude/panel.rs
git commit -m "refactor(terminal): make TerminalHandle::resize take &self"
```

---

## Task 2: Terminal registry + tab binding on `Editor`

**Files:**
- Modify: `helix-view/src/editor.rs`

- [ ] **Step 1: Add the `TerminalId` key type and imports**

Near the top of `helix-view/src/editor.rs` (with the other `use`/type declarations):

```rust
slotmap::new_key_type! {
    /// Editor-local handle to a standalone (`:terminal`) embedded terminal,
    /// stable across removals — mirrors `AgentSessionId` for agents.
    pub struct TerminalId;
}

/// What a terminal-backed buffer resolves to.
#[derive(Clone, Copy, Debug)]
pub enum TerminalRef {
    /// An agent session's terminal (owned by `Editor::agents`).
    Agent(crate::agent::AgentSessionId),
    /// A standalone terminal (owned by `Editor::terminals`).
    Standalone(TerminalId),
}
```

- [ ] **Step 2: Add the registry + map fields to `Editor`**

In `pub struct Editor { ... }` (near `pub agents: crate::agent::AgentRegistry,` at ~line 1382):

```rust
    /// Standalone embedded terminals (the `:terminal` command), kept alive
    /// independently of any view so a terminal survives closing its tab.
    pub terminals: slotmap::SlotMap<TerminalId, crate::terminal::TerminalHandle>,
    /// Maps a host document to the terminal it displays. A host document is a
    /// read-only, empty scratch buffer that exists only to make a terminal a
    /// navigable buffer (gp/gn, splits, bufferline).
    pub terminal_docs: std::collections::HashMap<DocumentId, TerminalRef>,
```

- [ ] **Step 3: Initialize the new fields**

In `Editor::new` (where `documents: BTreeMap::new(),` etc. are set, ~line 1527):

```rust
            terminals: slotmap::SlotMap::with_key(),
            terminal_docs: std::collections::HashMap::new(),
```

- [ ] **Step 4: Build**

Run: `cargo build -p helix-view`
Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/editor.rs
git commit -m "feat(editor): add standalone terminal registry and terminal_docs map"
```

---

## Task 3: `resolve_terminal`, `is_terminal_doc`, `focused_terminal` helpers

**Files:**
- Modify: `helix-view/src/editor.rs`

- [ ] **Step 1: Add resolution helpers to `impl Editor`**

Place near the document helpers (after `new_document`, ~line 2158):

```rust
    /// Resolve a terminal reference to its live handle, if it still exists.
    pub fn resolve_terminal(&self, r: &TerminalRef) -> Option<&crate::terminal::TerminalHandle> {
        match r {
            TerminalRef::Agent(id) => self.agents.get(*id).map(|s| &s.terminal),
            TerminalRef::Standalone(id) => self.terminals.get(*id),
        }
    }

    /// True when `doc_id` is a terminal-backed host document.
    pub fn is_terminal_doc(&self, doc_id: DocumentId) -> bool {
        self.terminal_docs.contains_key(&doc_id)
    }

    /// The terminal handle behind the currently focused view, if that view
    /// shows a terminal buffer.
    pub fn focused_terminal(&self) -> Option<&crate::terminal::TerminalHandle> {
        let doc_id = self.tree.get(self.tree.focus).doc;
        let r = self.terminal_docs.get(&doc_id)?;
        self.resolve_terminal(r)
    }
```

- [ ] **Step 2: Build**

Run: `cargo build -p helix-view`
Expected: compiles clean. (If `self.tree.get`/`self.tree.focus` names differ, confirm against `helix-view/src/tree.rs` — `Tree::get(&self, ViewId) -> &View` and `pub focus: ViewId`.)

- [ ] **Step 3: Commit**

```bash
git add helix-view/src/editor.rs
git commit -m "feat(editor): terminal resolution helpers (resolve/is_terminal_doc/focused_terminal)"
```

---

## Task 4: Open-as-tab helpers on `Editor`

Create the host document, register the mapping, and switch to it.

**Files:**
- Modify: `helix-view/src/editor.rs`

- [ ] **Step 1: Add `open_terminal_tab` and `open_agent_tab`**

In `impl Editor`, after the helpers from Task 3:

```rust
    /// Register `handle` as a standalone terminal and open it in a new host
    /// buffer, switching the current view to it. Returns the host document id.
    pub fn open_terminal_tab(&mut self, handle: crate::terminal::TerminalHandle) -> DocumentId {
        let term_id = self.terminals.insert(handle);
        self.open_terminal_ref(TerminalRef::Standalone(term_id))
    }

    /// Open an existing agent session as a host buffer (tab), switching the
    /// current view to it. Reuses an existing tab for that session if present.
    pub fn open_agent_tab(&mut self, session: crate::agent::AgentSessionId) -> DocumentId {
        if let Some((doc_id, _)) = self
            .terminal_docs
            .iter()
            .find(|(_, r)| matches!(r, TerminalRef::Agent(id) if *id == session))
        {
            let doc_id = *doc_id;
            self.switch(doc_id, Action::Replace);
            return doc_id;
        }
        self.open_terminal_ref(TerminalRef::Agent(session))
    }

    /// Shared: create a read-only empty host document bound to `r`, then switch.
    fn open_terminal_ref(&mut self, r: TerminalRef) -> DocumentId {
        let mut doc = Document::default(self.config.clone(), self.syn_loader.clone());
        doc.readonly = true;
        let doc_id = self.new_document(doc);
        self.terminal_docs.insert(doc_id, r);
        self.switch(doc_id, Action::Replace);
        doc_id
    }
```

Notes for the implementer:
- `Document::default(config, syn_loader)` is the existing scratch-buffer constructor (see `Document::default`, `document.rs:774`).
- `doc.readonly` is the existing field used by read-only buffers; confirm the field name in `document.rs` (it is `pub readonly: bool`). If it is instead behind a setter, use that.
- `Action::Replace` is the existing enum variant used by `goto_buffer`/`switch`.

- [ ] **Step 2: Build**

Run: `cargo build -p helix-view`
Expected: compiles clean.

- [ ] **Step 3: Commit**

```bash
git add helix-view/src/editor.rs
git commit -m "feat(editor): open_terminal_tab/open_agent_tab host-document helpers"
```

---

## Task 5: Clean up mapping when a terminal tab closes

Closing the view/document must drop the map entry but **not** the registry handle (agent stays; standalone terminal stays in `editor.terminals`, reopenable). Hook the existing `DocumentDidClose` event.

**Files:**
- Modify: `helix-view/src/editor.rs` (inside `close_document`, or via the `DocumentDidClose` dispatch)

- [ ] **Step 1: Remove the map entry on close**

In `Editor::close_document` (~line 2267), after the document is removed from `self.documents`, add:

```rust
        // A terminal host document: drop its binding. The underlying handle is
        // owned by a registry (agents / terminals) and intentionally survives.
        self.terminal_docs.remove(&doc_id);
```

(Place it near the existing per-close cleanup so it runs on every path that removes the document.)

- [ ] **Step 2: Build**

Run: `cargo build -p helix-view`
Expected: compiles clean.

- [ ] **Step 3: Commit**

```bash
git add helix-view/src/editor.rs
git commit -m "feat(editor): drop terminal_docs binding on document close (handle survives)"
```

---

## Task 6: Render the terminal grid for terminal-doc views

**Files:**
- Modify: `helix-term/src/ui/editor.rs` (`render_view`, top of function ~line 137)

- [ ] **Step 1: Add the render branch**

At the very start of `render_view`, after `let inner = view.inner_area(doc); let area = view.area;`, before any text/syntax setup:

```rust
        // Terminal buffers: blit the emulator grid and skip all text machinery
        // (gutters, syntax, diagnostics, selections don't apply).
        if let Some(term_ref) = editor.terminal_docs.get(&view.doc).copied() {
            surface.set_style(inner, editor.theme.get("ui.background"));
            if let Some(handle) = editor.resolve_terminal(&term_ref) {
                handle.resize(inner.height, inner.width);
                let _ = crate::ui::terminal::render(handle, inner, surface);
            }
            // Right border between splits, matching the text path below.
            if viewport.right() != view.area.right() {
                // (reuse the same border draw the normal path uses at the end of
                // render_view; extract it into `fn draw_split_border(surface, view, theme)`
                // and call it here as well as at the bottom of render_view.)
                Self::draw_split_border(surface, view, &editor.theme);
            }
            return;
        }
```

- [ ] **Step 2: Extract the split-border draw into a helper**

The existing code at the end of `render_view` ("if we're not at the edge of the screen, draw a right border", ~line 271) should be lifted into:

```rust
    fn draw_split_border(surface: &mut Surface, view: &View, theme: &Theme) {
        // move the existing right-border drawing body here verbatim
    }
```

and called both at the end of `render_view` (replacing the inline block) and from the terminal branch above. If the border body references locals not available here, keep it minimal (the vertical separator using `theme.get("ui.window")`).

- [ ] **Step 3: Build**

Run: `cargo build -p helix-term`
Expected: compiles clean.

- [ ] **Step 4: Manual check (grid renders)**

This is UI-integration code (not unit-testable). Defer full manual verification to Task 11, but a quick smoke test: `cargo build --release -p helix-term` succeeds.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/editor.rs
git commit -m "feat(ui): render terminal grid for terminal-doc views"
```

---

## Task 7: Terminal cursor in Insert mode

**Files:**
- Modify: `helix-term/src/ui/editor.rs` (`Component::cursor`, ~line 1899)

- [ ] **Step 1: Special-case the focused terminal view**

At the top of `cursor`, before `match editor.cursor()`:

```rust
        // A focused terminal buffer shows the emulator cursor only while the
        // user is driving it (Insert mode); otherwise it's hidden.
        let focus_doc = editor.tree.get(editor.tree.focus).doc;
        if let Some(term_ref) = editor.terminal_docs.get(&focus_doc).copied() {
            if editor.mode() == Mode::Insert {
                if let Some(handle) = editor.resolve_terminal(&term_ref) {
                    let view = editor.tree.get(editor.tree.focus);
                    let doc = editor.document(focus_doc).unwrap();
                    let inner = view.inner_area(doc);
                    if let Some((row, col)) = handle.snapshot().cursor {
                        let pos = Position::new(
                            inner.y as usize + row as usize,
                            inner.x as usize + col as usize,
                        );
                        return (Some(pos), CursorKind::Block);
                    }
                }
            }
            return (None, CursorKind::Hidden);
        }
```

Notes:
- `handle.snapshot().cursor` is `Option<(u16, u16)>` (row, col) — see `terminal.rs::snapshot`.
- `Mode` and `Position` are already imported in this file (used elsewhere).

- [ ] **Step 2: Build**

Run: `cargo build -p helix-term`
Expected: compiles clean.

- [ ] **Step 3: Commit**

```bash
git add helix-term/src/ui/editor.rs
git commit -m "feat(ui): show terminal cursor for focused terminal buffer in insert mode"
```

---

## Task 8: Modal input routing (Insert-mode keys → PTY; Shift-Esc)

**Files:**
- Test: `helix-term/src/ui/terminal.rs` (extend existing `tests` module)
- Modify: `helix-term/src/ui/editor.rs` (`Event::Key` arm, ~line 1665)

- [ ] **Step 1: Add a failing test for Shift-Esc byte encoding**

Confirm the byte we forward for a plain `ESC` is `0x1b` (used for Shift-Esc). In `helix-term/src/ui/terminal.rs` `mod tests`:

```rust
    #[test]
    fn plain_escape_byte_is_1b() {
        // Shift-Esc forwards a bare ESC to the PTY.
        assert_eq!(encode_key(&key(KeyCode::Esc, KeyModifiers::NONE)), Some(vec![0x1b]));
    }
```

- [ ] **Step 2: Run it**

Run: `cargo test -p helix-term --lib ui::terminal::tests::plain_escape_byte_is_1b`
Expected: PASS (this documents existing `encode_key` behavior we rely on; if it fails, `encode_key` changed and the intercept below must special-case ESC explicitly — it already does).

- [ ] **Step 3: Add the intercept in the `Event::Key` arm**

Immediately inside `Event::Key(mut key) => {` after `canonicalize_key(&mut key);` and `cx.editor.status_msg = None;`, before `let mode = cx.editor.mode();`:

```rust
                // Terminal buffers, modal input: in Insert mode, forward keys to
                // the PTY instead of the editor's insert machinery. Plain Esc is
                // NOT intercepted, so it falls through to the keymap and exits
                // insert mode (back to editor/Normal). Shift-Esc (or any modified
                // Esc) sends a real ESC to the terminal.
                if cx.editor.mode() == Mode::Insert && cx.editor.focused_terminal().is_some() {
                    let plain_esc =
                        key.code == KeyCode::Esc && key.modifiers.is_empty();
                    if !plain_esc {
                        let bytes = if key.code == KeyCode::Esc {
                            Some(vec![0x1b])
                        } else {
                            crate::ui::terminal::encode_key(&key)
                        };
                        if let Some(bytes) = bytes {
                            if let Some(handle) = cx.editor.focused_terminal() {
                                handle.write_input(&bytes);
                            }
                        }
                        return EventResult::Consumed(None);
                    }
                }
```

Notes:
- `KeyCode`, `KeyModifiers` are already imported in this file.
- Normal-mode keys are untouched → `gp`/`gn`, `Ctrl-w`, `:`, and `i`/`a` (enter Insert) all work. Because the host doc is `readonly`, `i`/`a` enter Insert without editing; other edit commands are inert.

- [ ] **Step 4: Build + run terminal tests**

Run: `cargo build -p helix-term && cargo test -p helix-term --lib ui::terminal`
Expected: compiles; all terminal tests PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/editor.rs helix-term/src/ui/terminal.rs
git commit -m "feat(ui): modal input routing for terminal buffers (Insert->PTY, Shift-Esc)"
```

---

## Task 9: Mouse — wheel forwarding + selection in terminal tabs

**Files:**
- Modify: `helix-term/src/ui/editor.rs` (`handle_mouse_event`, ~line 1378)
- Modify: `helix-term/src/compositor.rs` (`handle_mouse_selection` gate)

- [ ] **Step 1: Forward the wheel over a terminal-doc view**

In `EditorView::handle_mouse_event`, at the start (after computing which view/coords the event is over, or before the default handling), add a branch. Use the existing view-at-coords resolution the function already performs; if the view under the pointer is a terminal-doc, forward the wheel and consume:

```rust
        use helix_view::input::MouseEventKind;
        if let MouseEventKind::ScrollUp | MouseEventKind::ScrollDown = event.kind {
            if let Some((view_id, _)) = cxt.editor.tree.views().find(|(v, _)| {
                v.area.contains(helix_core::Position::new(event.row as usize, event.column as usize).into())
            }).map(|(v, f)| (v.id, f)) {
                let doc_id = cxt.editor.tree.get(view_id).doc;
                if let Some(term_ref) = cxt.editor.terminal_docs.get(&doc_id).copied() {
                    if let Some(handle) = cxt.editor.resolve_terminal(&term_ref) {
                        let view = cxt.editor.tree.get(view_id);
                        let doc = cxt.editor.document(doc_id).unwrap();
                        let inner = view.inner_area(doc);
                        let col = event.column.saturating_sub(inner.x).min(inner.width.saturating_sub(1));
                        let row = event.row.saturating_sub(inner.y).min(inner.height.saturating_sub(1));
                        handle.wheel(matches!(event.kind, MouseEventKind::ScrollUp), col, row);
                        return EventResult::Consumed(None);
                    }
                }
            }
        }
```

Implementer note: use whatever view-hit-test helper `handle_mouse_event` already uses (it already maps clicks to views); the snippet above shows intent — prefer the existing `tree` hit-testing utility over re-implementing `contains`. Keep `TerminalHandle::wheel(up, col, row)` (already implemented).

- [ ] **Step 2: Give `handle_mouse_selection` access to the editor**

In `helix-term/src/compositor.rs`, change the signature so the gate can consult the focused view, and update the one caller in `handle_event`:

```rust
    // signature
    fn handle_mouse_selection(&mut self, event: &MouseEvent, editor: &Editor) -> Option<bool> {
```

```rust
    // caller, inside Compositor::handle_event's `if let Event::Mouse(mouse) = event` block
        if let Event::Mouse(mouse) = event {
            if let Some(consumed) = self.handle_mouse_selection(mouse, cx.editor) {
                return consumed;
            }
        }
```

`Editor` is already in scope in `compositor.rs` (`use helix_view::Editor;`).

- [ ] **Step 3: Relax the gate to include focused terminal buffers**

Replace the existing early gate at the top of `handle_mouse_selection`:

```rust
        // Engage screen-grid selection when an overlay is open OR the focused
        // editor view is a terminal buffer (so drag-select-copy works in tabs).
        let terminal_focused = editor.is_terminal_doc(editor.tree.get(editor.tree.focus).doc);
        if self.layers.len() <= 1 && !terminal_focused {
            self.mouse_select = None;
            return None;
        }
```

- [ ] **Step 4: Build**

Run: `cargo build -p helix-term`
Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/editor.rs helix-term/src/compositor.rs
git commit -m "feat(ui): wheel + drag-select-copy in terminal buffers"
```

---

## Task 10: Open commands + panel binding + bufferline/statusline naming

**Files:**
- Modify: `helix-view/src/editor.rs` (`doc_display_name` helper)
- Modify: `helix-term/src/ui/editor.rs` (`render_bufferline` uses the helper)
- Modify: `helix-term/src/commands/typed.rs` (`:terminal-tab`, `:agent-tab`)
- Modify: `helix-term/src/ui/claude/panel.rs` (`t` binding)

- [ ] **Step 1: `doc_display_name` on `Editor`**

```rust
    /// Human label for a document, terminal-aware. Used by the bufferline and
    /// statusline so terminal tabs read as `[agent N]` / `[term]`.
    pub fn doc_display_name(&self, doc_id: DocumentId) -> Option<String> {
        match self.terminal_docs.get(&doc_id)? {
            TerminalRef::Agent(id) => self
                .agents
                .get(*id)
                .map(|s| format!("[{}]", s.display_name)),
            TerminalRef::Standalone(_) => Some("[terminal]".to_string()),
        }
    }
```

- [ ] **Step 2: Bufferline uses it**

In `render_bufferline` (`helix-term/src/ui/editor.rs`, ~line 727), where `fname` is computed from the doc path, prefer the terminal label:

```rust
        let label = editor
            .doc_display_name(doc.id())
            .unwrap_or_else(|| {
                doc.path()
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str())
                    .unwrap_or(SCRATCH_BUFFER_NAME)
                    .to_string()
            });
        let text = format!(" {}{} ", label, if doc.is_modified() { "[+]" } else { "" });
```

(Adapt to the exact variable names in the current function; the point is to call `doc_display_name` first.)

- [ ] **Step 3: `:terminal-tab` and `:agent-tab` commands**

In `helix-term/src/commands/typed.rs`, add handlers near `open_terminal` (~line 3076):

```rust
fn open_terminal_tab(
    cx: &mut compositor::Context,
    args: Args,
    event: PromptEvent,
) -> anyhow::Result<()> {
    if event != PromptEvent::Validate {
        return Ok(());
    }
    let args: Vec<String> = args.into_iter().map(|s| s.to_string()).collect();
    let pane = ui::terminal::spawn_terminal(cx.editor, &args)?;
    // spawn_terminal returns a TerminalPane; take its handle for registry
    // ownership. (Add `TerminalPane::into_handle(self) -> TerminalHandle`.)
    let handle = pane.into_handle();
    cx.editor.open_terminal_tab(handle);
    Ok(())
}

fn open_agent_tab(
    cx: &mut compositor::Context,
    _args: Args,
    event: PromptEvent,
) -> anyhow::Result<()> {
    if event != PromptEvent::Validate {
        return Ok(());
    }
    match cx.editor.agents.focused {
        Some(id) => {
            cx.editor.open_agent_tab(id);
            Ok(())
        }
        None => anyhow::bail!("no focused agent session"),
    }
}
```

Add `TerminalPane::into_handle` in `helix-term/src/ui/terminal.rs`:

```rust
impl TerminalPane {
    /// Consume the pane, returning its terminal handle for registry ownership.
    pub fn into_handle(self) -> TerminalHandle {
        self.terminal
    }
}
```

Register both in the `TypableCommand` table (mirror the existing `"terminal"` entry at typed.rs:3243):

```rust
        TypableCommand {
            name: "terminal-tab",
            aliases: &[],
            doc: "Open a terminal in a new buffer/tab.",
            fun: open_terminal_tab,
            completer: CommandCompleter::none(),
            ..Default::default() // match the exact struct shape used by neighbors
        },
        TypableCommand {
            name: "agent-tab",
            aliases: &[],
            doc: "Open the focused Claude agent in a new buffer/tab.",
            fun: open_agent_tab,
            completer: CommandCompleter::none(),
            ..Default::default()
        },
```

Implementer note: copy the exact field set from the neighboring `"terminal"` command literal — do not guess fields. `spawn_terminal` currently returns `TerminalPane`; `into_handle` unwraps it.

- [ ] **Step 4: Panel `t` binding — open focused agent as a tab**

In `helix-term/src/ui/claude/panel.rs`, `handle_list_key`, add a `t` arm that closes the panel and opens the focused agent as a tab:

```rust
            key!('t') => {
                if let Some(id) = ctx.editor.agents.focused {
                    return EventResult::Consumed(Some(Box::new(move |compositor, cx| {
                        compositor.remove(ID);
                        cx.editor.open_agent_tab(id);
                    })));
                }
            }
```

Add `t` to the panel's key-hint string (the `hint` in `render`) so it's discoverable, e.g. append `· t tab`.

- [ ] **Step 5: Build + tests**

Run: `cargo build -p helix-term && cargo test -p helix-term --lib`
Expected: compiles; existing tests PASS.

- [ ] **Step 6: Commit**

```bash
git add helix-view/src/editor.rs helix-term/src/ui/editor.rs helix-term/src/commands/typed.rs helix-term/src/ui/terminal.rs helix-term/src/ui/claude/panel.rs
git commit -m "feat: :terminal-tab/:agent-tab commands, panel 't', terminal-aware bufferline"
```

---

## Task 11: End-to-end manual verification (release)

**Files:** none (verification only)

- [ ] **Step 1: Build release (what the user's `hx` wrapper runs)**

Run: `cargo build --release -p helix-term`
Expected: `Finished release`.

- [ ] **Step 2: Verify against the spec's acceptance list**

Launch `target/release/hx` and confirm:
1. `:agent-tab` (with an agent running) and `:terminal-tab` open a full-screen terminal buffer; bufferline/statusline show `[agent N]` / `[terminal]`.
2. Normal mode: `gp`/`gn` cycle the tab and file buffers; `Ctrl-w v` split shows the terminal live in both panes.
3. `i` → type into the terminal; plain `Esc` → back to Normal; `Shift-Esc` → the program receives ESC (e.g. cancels claude's action).
4. Wheel scrolls the terminal tab; drag-select + copy works; buffers behind splits are unaffected.
5. `:bc` closes the tab; the agent is still listed in the floating panel; `:agent-tab` reopens the same live session.
6. The floating agent panel and floating `:terminal` still behave exactly as before.

- [ ] **Step 3: Commit any fixups**

```bash
git add -A && git commit -m "fix: phase-1 terminal-buffer verification fixups"
```

---

## Self-Review Notes (author)

- **Spec coverage:** data model (T2), resize-for-render (T1), render branch (T6), cursor (T7), modal input incl. Shift-Esc (T8), open/close lifecycle (T4/T5), bufferline naming (T10), mouse wheel + selection (T9), open commands + panel binding (T10). Event bus + toast/tab-focus retarget are **Phase 2** (out of scope here, per header).
- **Known implementer confirmations** (call these out in review, don't guess): exact `Tree` accessors (`tree.get`, `tree.focus`), `Document.readonly` field name, the `TypableCommand` struct field set, and the precise view-hit-test helper in `handle_mouse_event`. Each is flagged inline where it occurs.
- **Type consistency:** `TerminalRef` (Agent/Standalone), `resolve_terminal`, `is_terminal_doc`, `focused_terminal`, `open_terminal_tab`, `open_agent_tab`, `doc_display_name`, `TerminalPane::into_handle`, `TerminalHandle::resize(&self)`, `TerminalHandle::wheel` used consistently across tasks.
