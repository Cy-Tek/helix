# Capsule Statusline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the approved optional capsule statusline and bufferline/titlebar style.

**Architecture:** Add opt-in statusline style/glyph configuration in `helix-view`, then branch the existing terminal renderers in `helix-term` to either classic or capsule rendering. Capsule V1 uses an opinionated layout with small helper functions so future configurable segment groups and the information-rich variant can be added without replacing the renderer.

**Tech Stack:** Rust, Helix `helix-view` config, `helix-term` TUI buffer rendering, TOML/Serde config parsing, cargo unit tests.

---

## File Structure

- Modify `helix-view/src/editor.rs`: add `StatusLineStyle` and `StatusLineGlyphs` enums plus fields on `StatusLineConfig`.
- Modify `helix-term/src/ui/statusline.rs`: add capsule glyph definitions, style helpers, segment rendering helpers, and a `render_capsule` branch.
- Modify `helix-term/src/ui/editor.rs`: branch `render_bufferline` to a capsule titlebar when statusline style is capsule.
- Modify `book/src/editor.md`: document `style` and `glyphs` statusline keys.
- Test in `helix-view/src/editor.rs`: config defaults and TOML parsing.
- Test in `helix-term/src/ui/statusline.rs`: glyph selection and capsule footer segment output using pure helper functions.
- Test in `helix-term/src/ui/editor.rs`: capsule titlebar helper output using pure helper functions.

## Task 1: Config Surface

**Files:**
- Modify: `helix-view/src/editor.rs`
- Test: `helix-view/src/editor.rs`

- [x] Add failing tests under the existing `#[cfg(test)]` module:
  - `statusline_defaults_to_classic_nerd_glyphs`
  - `statusline_parses_capsule_style_and_plain_glyphs`
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-view statusline_ --lib`; expect compile failure for missing fields/types.
- [x] Add `StatusLineStyle` enum with `Classic` and `Capsule`, default `Classic`.
- [x] Add `StatusLineGlyphs` enum with `Nerd` and `Plain`, default `Nerd`.
- [x] Add `style` and `glyphs` fields to `StatusLineConfig`.
- [x] Re-run the focused helix-view tests; expect pass.

## Task 2: Capsule Footer Renderer

**Files:**
- Modify: `helix-term/src/ui/statusline.rs`
- Test: `helix-term/src/ui/statusline.rs`

- [x] Add failing helper tests:
  - `capsule_nerd_glyphs_use_powerline_caps_and_stars`
  - `capsule_plain_glyphs_avoid_nerd_font_symbols`
  - `capsule_footer_segments_include_mode_language_branch_and_position`
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term ui::statusline::tests`; expect compile failure for missing helpers.
- [x] Add `CapsuleGlyphs`, `capsule_glyphs`, `capsule_text`, and `capsule_separator` helpers.
- [x] Add helper functions that build footer left/right segment labels from mode, language, diagnostics summary, branch, position, and percentage.
- [x] Add `render_capsule` and call it from `render` when `config.statusline.style == Capsule`.
- [x] Re-run focused statusline tests; expect pass.

## Task 3: Capsule Titlebar / Bufferline

**Files:**
- Modify: `helix-term/src/ui/editor.rs`
- Test: `helix-term/src/ui/editor.rs`

- [x] Add failing helper tests:
  - `capsule_titlebar_segments_bold_project_and_file`
  - `capsule_titlebar_plain_glyphs_avoid_nerd_font_symbols`
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term ui::editor::tests::capsule_`; expect compile failure for missing helpers.
- [x] Add capsule titlebar helper structs/functions near `render_bufferline`.
- [x] Branch `render_bufferline` to capsule rendering when `editor.config().statusline.style == Capsule`.
- [x] Keep existing bufferline visibility rules unchanged.
- [x] Re-run focused editor UI tests; expect pass.

## Task 4: Docs, Integration Checks, And Release Build

**Files:**
- Modify: `book/src/editor.md`
- Verify all touched code.

- [x] Update `[editor.statusline]` docs with `style` and `glyphs`.
- [x] Run `cargo fmt --check`; fix formatting if needed.
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-view statusline_ --lib`.
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term ui::statusline`.
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo test --locked --offline -p helix-term ui::editor::tests::capsule_`.
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo check --locked --offline -p helix-term`.
- [x] Run `git diff --check`.
- [x] Run `HELIX_DISABLE_AUTO_GRAMMAR_BUILD=1 cargo build --release --bin hx --locked --offline`.
- [x] Verify `/Users/cy-tek/.local/bin/hx --version` after commit/rebuild.
