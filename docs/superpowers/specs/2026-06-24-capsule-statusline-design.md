# Capsule Statusline And Titlebar Design

## Summary

Add an optional capsule visual style for Helix's statusline and bufferline/titlebar chrome. The default Helix UI remains unchanged unless the user enables the new style. The first implementation targets the "Balanced Combo" design: a title-forward capsule header and a star-separated capsule footer, with a clear path to evolve toward a denser information-rich variant later.

## Goals

- Make the bottom statusline feel more aesthetically rich while preserving scan speed.
- Make the project name and current file/title visually stronger when the bufferline is visible.
- Use terminal-friendly capsule shapes, star separators, and bold/color accents.
- Keep the feature opt-in and compatible with existing Helix configuration.
- Provide a plain fallback glyph set so unsupported Nerd Font symbols do not make the UI unreadable.

## Non-Goals

- Do not replace the default classic statusline.
- Do not expose fully configurable capsule segment layouts in V1.
- Do not introduce a new top-bar visibility setting separate from `editor.bufferline`.
- Do not require every theme to define capsule-specific style keys.

## Configuration

The new statusline style is configured under `[editor.statusline]`:

```toml
[editor.statusline]
style = "capsule" # default: "classic"
glyphs = "nerd"   # alternatives: "plain"
```

`style = "classic"` preserves the existing renderer. `style = "capsule"` enables the new bottom statusline renderer and also changes the bufferline/titlebar rendering when `editor.bufferline` already makes the top bar visible.

`glyphs = "nerd"` uses rounded powerline-style capsule caps and icons. `glyphs = "plain"` uses portable bracket/paren-style boundaries and ASCII-safe separators.

## Theme Keys

Capsule-specific theme keys are additive. If a key is missing, rendering falls back to existing statusline or bufferline styles.

```toml
"ui.statusline.capsule" = { fg = "status_fg", bg = "status_bg" }
"ui.statusline.capsule.mode" = { fg = "mode_fg", bg = "mode_bg" }
"ui.statusline.capsule.file" = { fg = "file_fg", bg = "file_bg" }
"ui.statusline.capsule.project" = { fg = "project_fg", bg = "project_bg" }
"ui.statusline.capsule.meta" = { fg = "meta_fg", bg = "meta_bg" }
"ui.statusline.capsule.accent" = { fg = "accent" }
```

The cap glyphs must use the same background color as the surrounding terminal line and the same foreground color as the capsule body background. This makes the cap glyphs visually read as rounded ends.

## Bottom Statusline Layout

Capsule style uses an opinionated V1 layout instead of directly honoring the current `left`, `center`, and `right` lists. The implementation should still be structured around small render helpers so future work can allow configurable segment groups without rewriting the renderer.

Left cluster:

- mode capsule
- star separator
- language/file-type capsule
- star separator
- diagnostics capsule, only when diagnostics exist

Right cluster:

- git branch capsule, when version-control information is available
- star separator
- cursor position and position percentage capsule

The middle stays empty in V1. A future information-rich variant may add encoding, line ending, indentation, selection, register, LSP, or workspace diagnostic segments.

## Top Titlebar / Bufferline Layout

The top capsule titlebar appears only when Helix's existing `editor.bufferline` setting would already render a top bar.

Left cluster:

- bold project capsule
- bold, color-accented current file capsule
- muted parent path text

Right cluster:

- saved/modified/read-only state capsule
- LSP/indexing/spinner capsule when useful

The project title and file title should use bold styling. The file title should also use a distinct color accent so it reads as the main tab name.

## Glyph Sets

Nerd glyph set:

- left capsule cap: ``
- right capsule cap: ``
- file icon: `󰈙`
- git branch icon: ``
- primary separator/accent: `✦`

Plain glyph set:

- capsule boundaries use parentheses or brackets, for example `( NORMAL )`
- separator/accent uses `*`
- icons use text labels such as `file` and `git`

## Error Handling And Fallbacks

- Missing capsule theme keys fall back to existing `ui.statusline`, `ui.statusline.normal`, `ui.statusline.insert`, `ui.statusline.select`, `ui.bufferline`, and `ui.bufferline.active` styles.
- Missing project names fall back to the current workspace/root name behavior.
- Missing file paths fall back to the scratch buffer name.
- Missing version-control data simply omits the branch capsule.
- Diagnostics are omitted when no configured diagnostic counts are nonzero.

## Testing

Add unit-level render tests around:

- statusline config deserialization for `style` and `glyphs`
- default config preserving classic behavior
- capsule glyph set selection
- bottom capsule output containing expected mode, language, diagnostics, branch, and position segments
- plain fallback output avoiding Nerd Font glyphs
- bufferline/titlebar capsule output when bufferline is visible

Also run the existing `helix-term` check and rebuild the release `hx` binary after implementation.

## Future Direction

The next visual step, if desired, is the information-rich capsule variant. That version would keep the same capsule and star language but add more explicit data segments: encoding, line ending, indentation, selection count, register, LSP count/status, and workspace diagnostics.
