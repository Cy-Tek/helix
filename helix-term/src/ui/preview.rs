use crate::ui::{
    document::{render_document, LinePos, TextRenderer},
    picker::Preview,
    text_decorations::DecorationManager,
    EditorView,
};
use helix_core::{char_idx_at_visual_offset, text_annotations::TextAnnotations};
use helix_view::{graphics::Rect, view::ViewPosition, Editor};
use tui::{
    buffer::Buffer as Surface,
    terminal::{MediaCommand, MediaImage},
    widgets::{Block, Widget},
};

const PREVIEW_IMAGE_ID: u32 = 1;

pub(crate) fn preview_content_area(area: Rect) -> Rect {
    let inner = Block::bordered().inner(area);
    inner.inner(helix_view::graphics::Margin::horizontal(1))
}

pub(crate) fn render_preview(
    preview: Preview<'_, '_>,
    range: Option<(usize, usize)>,
    area: Rect,
    surface: &mut Surface,
    editor: &Editor,
    supports_kitty_graphics: bool,
    media: &mut Vec<MediaCommand>,
) {
    let background = editor.theme.get("ui.background");
    let text = editor.theme.get("ui.text");
    let directory = editor.theme.get("ui.text.directory");
    surface.clear_with(area, background);

    const BLOCK: Block<'_> = Block::bordered();
    let inner = preview_content_area(area);
    BLOCK.render(area, surface);

    if let Some(image) = preview.image() {
        if supports_kitty_graphics {
            media.push(MediaCommand::Image(MediaImage {
                id: PREVIEW_IMAGE_ID,
                area: image.area,
                width: image.width,
                height: image.height,
                payload_hash: image.payload_hash,
                png: image.png.clone(),
            }));
        } else {
            let alt_text = "<Image preview unavailable>";
            let x = inner.x + inner.width.saturating_sub(alt_text.len() as u16) / 2;
            let y = inner.y + inner.height / 2;
            surface.set_stringn(x, y, alt_text, inner.width as usize, text);
        }
        return;
    }

    let doc = match preview.document() {
        Some(doc)
            if range.is_none_or(|(start, end)| start <= end && end <= doc.text().len_lines()) =>
        {
            doc
        }
        _ => {
            if let Some(dir_content) = preview.dir_content() {
                for (i, (path, is_dir)) in
                    dir_content.iter().take(inner.height as usize).enumerate()
                {
                    let style = if *is_dir { directory } else { text };
                    surface.set_stringn(
                        inner.x,
                        inner.y + i as u16,
                        path,
                        inner.width as usize,
                        style,
                    );
                }
                return;
            }

            let alt_text = preview.placeholder();
            let x = inner.x + inner.width.saturating_sub(alt_text.len() as u16) / 2;
            let y = inner.y + inner.height / 2;
            surface.set_stringn(x, y, alt_text, inner.width as usize, text);
            return;
        }
    };

    let mut offset = ViewPosition::default();
    if let Some((start_line, end_line)) = range {
        let height = end_line - start_line;
        let text = doc.text().slice(..);
        let start = text.line_to_char(start_line);
        let middle = text.line_to_char(start_line + height / 2);
        if height < inner.height as usize {
            let text_fmt = doc.text_format(inner.width, None);
            let annotations = TextAnnotations::default();
            (offset.anchor, offset.vertical_offset) = char_idx_at_visual_offset(
                text,
                middle,
                -(inner.height as isize / 2),
                0,
                &text_fmt,
                &annotations,
            );
            if start < offset.anchor {
                offset.anchor = start;
                offset.vertical_offset = 0;
            }
        } else {
            offset.anchor = start;
        }
    }

    let loader = editor.syn_loader.load();
    let config = editor.config();

    let syntax_highlighter =
        EditorView::doc_syntax_highlighter(doc, offset.anchor, area.height, &loader);
    let mut overlay_highlights = Vec::new();
    if doc
        .language_config()
        .and_then(|config| config.rainbow_brackets)
        .unwrap_or(config.rainbow_brackets)
    {
        if let Some(overlay) = EditorView::doc_rainbow_highlights(
            doc,
            offset.anchor,
            area.height,
            &editor.theme,
            &loader,
        ) {
            overlay_highlights.push(overlay);
        }
    }

    EditorView::doc_diagnostics_highlights_into(doc, &editor.theme, &mut overlay_highlights);

    let mut decorations = DecorationManager::default();

    if let Some((start, end)) = range {
        let style = editor
            .theme
            .try_get("ui.highlight")
            .unwrap_or_else(|| editor.theme.get("ui.selection"));
        let draw_highlight = move |renderer: &mut TextRenderer, pos: LinePos| {
            if (start..=end).contains(&pos.doc_line) {
                let area = Rect::new(
                    renderer.viewport.x,
                    pos.visual_line,
                    renderer.viewport.width,
                    1,
                );
                renderer.set_style(area, style)
            }
        };
        decorations.add_decoration(draw_highlight);
    }

    render_document(
        surface,
        inner,
        doc,
        offset,
        &TextAnnotations::default(),
        syntax_highlighter,
        overlay_highlights,
        &editor.theme,
        decorations,
    );
}
