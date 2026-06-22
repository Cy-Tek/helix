# Kitty Image Preview Design

## Summary

Add in-process image previews to Helix picker file previews using the Kitty graphics protocol when the terminal supports it, with safe text fallbacks when graphics are unavailable.

## Goal

The file picker and file explorer should show real image previews for common web and game-development assets. The feature must not depend on external preview binaries. It should preserve the existing text, directory, large-file, binary, and missing-file preview behavior for non-image paths.

## Scope

The first implementation slice covers local static image files selected in existing picker previews:

- `png`
- `jpg` and `jpeg`
- `gif`, rendered as a static first frame
- `webp`, rendered as a static frame
- any additional static formats that come naturally from the same decoder without adding separate behavior

Animation, video, PDF rendering, remote URLs, and inline image rendering inside normal text buffers are out of scope for this slice.

## User Experience

When a selected picker item points to a supported image file and the preview pane is visible, Helix renders the image inside the existing preview pane. The image is scaled to fit within the bordered preview area while preserving aspect ratio. Empty space around the image remains normal themed background.

If the terminal does not support Kitty graphics, or if image decoding fails, the preview pane shows a concise placeholder instead of trying to open an external program. Existing preview toggling with `Ctrl-t` continues to work.

## Architecture

The picker preview cache gains an image preview variant alongside the existing document, directory, binary, large-file, and not-found variants. Image detection happens before binary-text rejection for files whose extension or header matches a supported image type, so normal image files are not reduced to the generic binary placeholder.

Image decoding stays in `helix-term`, close to picker preview logic. The first slice should use a mature Rust decoder crate with explicit support for PNG, JPEG, GIF, and WebP. The cached image preview stores the original dimensions plus a scaled PNG payload ready for Kitty transmission. The cache avoids repeatedly decoding and scaling the same selected asset while the picker remains open.

Terminal drawing support belongs in `helix-tui`, because Kitty graphics escape sequences must be emitted by the backend rather than written into the normal cell buffer. The backend sends the scaled PNG payload through Kitty graphics data transfer rather than relying on file-path references, so previews also work for paths with awkward characters and do not depend on terminal-side filesystem access. The backend exposes a small media API for:

- detecting whether Kitty image preview is enabled or supported
- rendering an image into a cell rectangle
- clearing a previously rendered image

The picker remains responsible for deciding which preview should be shown and for reserving/clearing the cell area. The terminal backend remains responsible for protocol-specific escape sequence formatting and output.

## Rendering Lifecycle

Kitty images are outside Helix's ordinary diffed cell buffer, so lifecycle handling is part of the feature rather than a polish item.

The implementation clears or replaces the active preview image when:

- the picker selection changes to another preview target
- preview visibility is toggled off
- the preview pane moves or resizes
- a non-image preview is rendered in the same pane
- the picker overlay closes or is dropped
- the terminal is cleared or restored

The picker also draws ordinary background cells over the preview area before or alongside graphics updates so unsupported terminals and test backends still have a coherent textual screen state.

## Configuration

The first slice is automatic and adds no user-facing configuration. The backend enables image previews only when it can confidently use Kitty graphics. Unsupported terminals keep the existing picker layout and show the image placeholder.

The feature should not add per-format configuration, external command templates, or preview quality knobs in the initial slice.

## Error Handling

Failures should be non-fatal and localized to the preview pane:

- unsupported terminal: show `<Image preview unavailable>`
- unsupported or undecodable image: show `<Unsupported image>`
- oversized image or decode limit exceeded: show `<Image too large to preview>`
- missing path: keep the existing `<File not found>` behavior

Errors should not prevent opening the file or navigating the picker.

## Testing

Unit tests should cover image-type classification, preview-cache selection, placeholder fallback behavior, and lifecycle state transitions that request clears when the active image changes or disappears.

Backend tests should use the test backend to record media operations without emitting terminal escapes. They should verify that rendering an image preview produces a media render request for the expected rectangle and that switching away from an image produces a clear request.

Manual verification should include a real Kitty-compatible terminal path using a small PNG and JPEG in the file picker, plus a fallback terminal or disabled mode to confirm placeholders remain readable.

## Implementation Bias

The design preference is reliability inside Helix over minimizing code at the cost of lifecycle glitches. If a terminal support probe is inconclusive, the first implementation should fall back to text placeholders rather than emitting graphics speculatively.
