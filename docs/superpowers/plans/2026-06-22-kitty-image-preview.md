# Kitty Image Preview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Add in-process Kitty graphics image previews to Helix file picker and file explorer previews.

**Architecture:** Picker rendering queues media commands through `compositor::Context`, and `application::render` flushes them after normal cell drawing. `helix-term` classifies, decodes, scales, and caches image previews; `helix-tui` owns Kitty graphics protocol emission and fallback behavior.

**Tech Stack:** Rust, Helix `Picker`, `helix-tui` backend/terminal abstraction, Kitty graphics protocol, `image` crate.

---

### Task 1: Terminal Media Primitives

**Files:**
- Modify: `helix-tui/Cargo.toml`
- Modify: `helix-tui/src/terminal.rs`
- Modify: `helix-tui/src/backend/mod.rs`
- Modify: `helix-tui/src/backend/test.rs`
- Modify: `helix-tui/src/backend/termina.rs`
- Test: `helix-tui/tests/terminal.rs`

- [x] **Step 1: Write failing media-operation tests**

Add tests in `helix-tui/tests/terminal.rs` that construct `Terminal<TestBackend>`, render one `MediaCommand::Image`, assert that the test backend recorded a render operation, then render an empty command list and assert a clear operation was recorded.

- [x] **Step 2: Run tests to verify RED**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-tui terminal_records_media_operations --test terminal`

Expected: compile failure because `MediaCommand` and `Terminal::draw_media` do not exist.

- [x] **Step 3: Implement terminal media API**

Add `MediaCommand`, `MediaImage`, and `MediaOperation` types in `helix-tui/src/terminal.rs`. Add backend image render/clear hooks with default no-op implementations, implement recording in `TestBackend`, and implement Kitty graphics escape output in `TerminaBackend` using chunked PNG payload transfer.

- [x] **Step 4: Run tests to verify GREEN**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-tui terminal_records_media_operations --test terminal`

Expected: one test passes.

### Task 2: Image Preview Decode And Scaling

**Files:**
- Modify: `helix-term/Cargo.toml`
- Create: `helix-term/src/ui/image_preview.rs`
- Modify: `helix-term/src/ui/mod.rs`

- [x] **Step 1: Write failing unit tests**

Add unit tests in `helix-term/src/ui/image_preview.rs` for `is_supported_image_path`, `decode_image_preview`, decode failure, and aspect-ratio-preserving scaling into a cell rectangle.

- [x] **Step 2: Run tests to verify RED**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-term ui::image_preview`

Expected: compile failure because the module and functions do not exist yet.

- [x] **Step 3: Implement decode helper**

Use the `image` crate with PNG, JPEG, GIF, and WebP support. Decode from bytes, thumbnail to fit the preview pixel budget derived from the cell rectangle, encode the scaled image as PNG bytes, and return original/scaled dimensions plus payload hash.

- [x] **Step 4: Run tests to verify GREEN**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-term ui::image_preview`

Expected: all image preview helper tests pass.

### Task 3: Picker Preview Integration

**Files:**
- Modify: `helix-term/src/compositor.rs`
- Modify: `helix-term/src/application.rs`
- Modify: `helix-term/src/commands.rs`
- Modify: `helix-term/src/handlers/auto_save.rs`
- Modify: `helix-term/src/ui/picker.rs`

- [x] **Step 1: Write failing picker tests**

Add picker-module tests that exercise preview classification: supported image paths become `CachedPreview::Image`, oversized images become `LargeFile`, undecodable supported image bytes become image-specific failure placeholders, and non-image binaries still become `Binary`.

- [x] **Step 2: Run tests to verify RED**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-term ui::picker`

Expected: compile failure or test failure because image cache variants and media command queuing do not exist.

- [x] **Step 3: Integrate media command queue**

Add `media: &mut Vec<tui::terminal::MediaCommand>` to `compositor::Context`, initialize it in each context construction, and call `self.terminal.draw_media(&media_commands)` in `Application::render` after `self.terminal.draw(pos, kind)`.

- [x] **Step 4: Integrate image cache and rendering**

Add `CachedPreview::Image` and `CachedPreview::UnsupportedImage`. In `get_preview`, detect supported image paths before binary rejection, decode and cache image previews within the existing max preview size, and render image previews by clearing the preview area and pushing a `MediaCommand::Image` with the inner preview rectangle.

- [x] **Step 5: Run tests to verify GREEN**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-term ui::picker`

Expected: picker tests pass.

### Task 4: Full Verification And Finish

**Files:**
- Modify: `helix-tui/src/terminal.rs`
- Modify: `helix-tui/src/backend/mod.rs`
- Modify: `helix-tui/src/backend/test.rs`
- Modify: `helix-tui/src/backend/termina.rs`
- Modify: `helix-term/src/ui/image_preview.rs`
- Modify: `helix-term/src/ui/picker.rs`
- Modify: `helix-term/src/compositor.rs`
- Modify: `helix-term/src/application.rs`
- Modify: `helix-term/src/commands.rs`
- Modify: `helix-term/src/handlers/auto_save.rs`

- [x] **Step 1: Format**

Run: `cargo fmt --all`

Expected: formatter exits 0.

- [x] **Step 2: Run focused tests**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-tui terminal --test terminal`

Expected: terminal tests pass.

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-term ui::image_preview`

Expected: image preview helper tests pass.

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test -p helix-term ui::picker`

Expected: picker tests pass.

- [x] **Step 3: Build Helix term**

Run: `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo check -p helix-term`

Expected: build check exits 0, with only pre-existing unrelated warnings if any.

- [x] **Step 4: Inspect diff**

Run: `git diff --stat && git diff --check`

Expected: changed files are limited to the design/plan and implementation files, and `git diff --check` exits 0.
