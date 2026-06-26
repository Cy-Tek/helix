//! A full-screen markdown preview overlay.
//!
//! The current document's markdown is parsed into an ordered list of blocks: runs of styled text
//! (reusing the existing [`Markdown`] renderer) interleaved with media blocks — mermaid diagrams
//! (rendered to images via [`mermaid`]) and standalone image links. Media blocks are placed using
//! the terminal's kitty graphics support through [`Context::media`]; text scrolls beneath them.

pub mod mermaid;

use std::{
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

use arc_swap::ArcSwap;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use tui::{
    buffer::Buffer as Surface,
    terminal::{MediaCommand, MediaImage},
    text::{Span, Text},
    widgets::{Block, Paragraph, Widget, Wrap},
};

use helix_core::syntax;
use helix_view::{
    graphics::{CursorKind, Margin, Rect, Style},
    theme::Modifier,
};

use crate::{
    compositor::{Component, Context, Event as CompositorEvent, EventResult},
    ctrl, key,
    ui::{
        image_preview::{decode_image_preview, ImagePreview, CELL_PIXEL_HEIGHT, CELL_PIXEL_WIDTH},
        kitty_graphics,
        markdown::Markdown,
        text::required_size,
    },
};

use self::mermaid::MermaidRenderer;

pub const ID: &str = "markdown-preview";

/// Base id for diagram/image placements. Offset by the block index so several diagrams can be on
/// screen at once without colliding with each other or with the picker/file-tree preview (id 1).
const IMAGE_ID_BASE: u32 = 1000;

/// An image embedded in the preview. The raw bytes are kept and scaled on demand (the natural
/// size is re-derived by the scaler), but we validate up front that they decode.
enum BlockImage {
    Ready { bytes: Vec<u8> },
    Error(String),
}

impl BlockImage {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        match image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format() {
            Ok(reader) => match reader.into_dimensions() {
                Ok((width, height)) if width > 0 && height > 0 => BlockImage::Ready { bytes },
                Ok(_) => BlockImage::Error("image has zero size".to_string()),
                Err(err) => BlockImage::Error(format!("invalid image: {err}")),
            },
            Err(err) => BlockImage::Error(format!("invalid image: {err}")),
        }
    }
}

struct MediaBlock {
    label: String,
    content: BlockImage,
}

enum PreviewBlock {
    /// A run of markdown text, re-parsed against the active theme on each render.
    Markdown(Markdown),
    /// A diagram or image.
    Media(MediaBlock),
}

pub struct MarkdownPreview {
    contents: String,
    base_dir: Option<PathBuf>,
    renderer: MermaidRenderer,
    config_loader: Arc<ArcSwap<syntax::Loader>>,
    title: String,
    max_width: u16,
    blocks: Vec<PreviewBlock>,
    scroll: u16,
    /// Scaled images keyed by (block index, target columns, target rows). `None` records a decode
    /// failure so it isn't retried every frame.
    image_cache: HashMap<(usize, u16, u16), Option<ImagePreview>>,
    /// Height of the scrollable content region from the last render, for half-page scrolling.
    last_viewport_height: u16,
}

impl MarkdownPreview {
    pub fn new(
        contents: String,
        base_dir: Option<PathBuf>,
        renderer: MermaidRenderer,
        config_loader: Arc<ArcSwap<syntax::Loader>>,
        title: String,
        max_width: u16,
    ) -> Self {
        let blocks = parse_blocks(
            &contents,
            base_dir.as_deref(),
            &renderer,
            &config_loader,
        );
        Self {
            contents,
            base_dir,
            renderer,
            config_loader,
            title,
            max_width: max_width.max(20),
            blocks,
            scroll: 0,
            image_cache: HashMap::new(),
            last_viewport_height: 0,
        }
    }

    /// Re-parse the source and re-render diagrams (e.g. after the file changed on disk).
    fn rebuild(&mut self) {
        self.blocks = parse_blocks(
            &self.contents,
            self.base_dir.as_deref(),
            &self.renderer,
            &self.config_loader,
        );
        self.image_cache.clear();
    }

    fn half_page(&self) -> u16 {
        (self.last_viewport_height / 2).max(1)
    }
}

/// What to draw for a block at render time, along with its height in rows.
enum Plan {
    /// A markdown text run; `index` points into `blocks`.
    Text { index: usize, height: u16 },
    /// A diagram/image to place via kitty graphics. `img_rows` is the image height; the block also
    /// reserves a row of padding above, a caption row, and a row of padding below.
    Image {
        index: usize,
        img_cols: u16,
        img_rows: u16,
        label: String,
        height: u16,
    },
    /// A short styled message (render error, or fallback when graphics are unavailable).
    Note {
        text: Text<'static>,
        style: Style,
        height: u16,
    },
}

impl Plan {
    fn height(&self) -> u16 {
        match self {
            Plan::Text { height, .. } | Plan::Image { height, .. } | Plan::Note { height, .. } => {
                *height
            }
        }
    }
}

impl Component for MarkdownPreview {
    fn handle_event(&mut self, event: &CompositorEvent, _ctx: &mut Context) -> EventResult {
        match event {
            CompositorEvent::Key(key) => match *key {
                key!('j') | key!(Down) => {
                    self.scroll = self.scroll.saturating_add(1);
                    EventResult::Consumed(None)
                }
                key!('k') | key!(Up) => {
                    self.scroll = self.scroll.saturating_sub(1);
                    EventResult::Consumed(None)
                }
                ctrl!('d') | key!(PageDown) | key!(' ') => {
                    self.scroll = self.scroll.saturating_add(self.half_page());
                    EventResult::Consumed(None)
                }
                ctrl!('u') | key!(PageUp) => {
                    self.scroll = self.scroll.saturating_sub(self.half_page());
                    EventResult::Consumed(None)
                }
                key!('g') | key!(Home) => {
                    self.scroll = 0;
                    EventResult::Consumed(None)
                }
                key!('G') | key!(End) => {
                    // Clamped to the real maximum during render.
                    self.scroll = u16::MAX;
                    EventResult::Consumed(None)
                }
                key!('r') => {
                    self.rebuild();
                    EventResult::Consumed(None)
                }
                key!('q') | key!(Esc) => EventResult::Consumed(Some(Box::new(|compositor, _| {
                    compositor.remove(ID);
                }))),
                _ => EventResult::Ignored(None),
            },
            CompositorEvent::Mouse(event) => {
                use helix_view::input::MouseEventKind;
                match event.kind {
                    MouseEventKind::ScrollDown => {
                        self.scroll = self.scroll.saturating_add(3);
                    }
                    MouseEventKind::ScrollUp => {
                        self.scroll = self.scroll.saturating_sub(3);
                    }
                    _ => {}
                }
                // Stay modal: swallow all mouse events.
                EventResult::Consumed(None)
            }
            CompositorEvent::Resize(..) => {
                // Cell geometry may have changed; drop cached scalings.
                self.image_cache.clear();
                EventResult::Ignored(None)
            }
            _ => EventResult::Ignored(None),
        }
    }

    fn render(&mut self, area: Rect, surface: &mut Surface, ctx: &mut Context) {
        // Resolve styles up front so we don't hold a borrow on the theme.
        let theme = &ctx.editor.theme;
        let background = theme.get("ui.popup");
        let border_style = theme.get("ui.window");
        let text_style = theme.get("ui.text");
        let title_style = text_style.add_modifier(Modifier::BOLD);
        let caption_style = text_style.add_modifier(Modifier::ITALIC);
        let muted_style = text_style.add_modifier(Modifier::DIM);
        let error_style = theme
            .try_get("error")
            .or_else(|| theme.try_get("diagnostic.error"))
            .unwrap_or(text_style);

        surface.clear_with(area, background);

        let title = format!(" {} — j/k scroll · r refresh · q close ", self.title);
        let block = Block::bordered()
            .border_style(border_style)
            .title(Span::styled(title, title_style));
        let inner = block.inner(area);
        block.render(area, surface);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // Centered content column inside horizontal gutters.
        let padded = inner.inner(Margin::horizontal(2));
        let content_width = padded.width.min(self.max_width);
        let content_x = padded.x + (padded.width.saturating_sub(content_width)) / 2;
        let viewport_height = padded.height;
        let content = Rect::new(content_x, padded.y, content_width, viewport_height);
        self.last_viewport_height = viewport_height;

        if content.width == 0 || content.height == 0 {
            return;
        }

        let (cell_w, cell_h) = ctx
            .cell_size_pixels
            .map(|(w, h)| (u32::from(w).max(1), u32::from(h).max(1)))
            .unwrap_or((CELL_PIXEL_WIDTH, CELL_PIXEL_HEIGHT));
        let supports_graphics = ctx.supports_kitty_graphics;

        // Disjoint borrows of distinct fields so the text parses can coexist with cache mutation.
        let blocks = &self.blocks;
        let cache = &mut self.image_cache;

        if blocks.is_empty() {
            let note = "This document has no previewable content.";
            draw_centered(
                surface,
                content,
                content.y + content.height / 2,
                note,
                muted_style,
            );
            return;
        }

        // Parse text blocks once; reused for both layout and drawing.
        let texts: Vec<Option<Text>> = blocks
            .iter()
            .map(|block| match block {
                PreviewBlock::Markdown(md) => Some(md.parse(Some(theme), Some(content_width))),
                PreviewBlock::Media(_) => None,
            })
            .collect();

        // Box images into at most the viewport height (minus a caption row). Diagrams taller than
        // this are scaled down to fit the width; partial rows are drawn while scrolling, so even a
        // tall diagram is fully viewable.
        let max_img_rows = viewport_height.saturating_sub(1).max(1);

        // Build the layout plan and total height.
        let mut plans: Vec<Plan> = Vec::with_capacity(blocks.len());
        let mut total: u32 = 0;
        for (index, block) in blocks.iter().enumerate() {
            let plan = match block {
                PreviewBlock::Markdown(_) => {
                    let text = texts[index].as_ref().expect("markdown block has text");
                    let height = required_size(text, content_width).1;
                    Plan::Text { index, height }
                }
                PreviewBlock::Media(media) => match &media.content {
                    BlockImage::Error(message) => {
                        let text = Text::from(format!("⚠ {} ({})", media.label, message));
                        let height = required_size(&text, content_width).1 + 1;
                        Plan::Note {
                            text,
                            style: error_style,
                            height,
                        }
                    }
                    BlockImage::Ready { .. } if supports_graphics => {
                        // Scale to fit (content_width × max_img_rows), preserving aspect, and use
                        // the resulting cell dimensions for both layout and placement.
                        match get_scaled(
                            cache,
                            blocks,
                            index,
                            content_width,
                            max_img_rows,
                            cell_w,
                            cell_h,
                        ) {
                            Some(preview) => Plan::Image {
                                index,
                                img_cols: preview.area.width,
                                img_rows: preview.area.height,
                                label: media.label.clone(),
                                // top padding + image rows + a caption row + a blank spacer
                                height: preview.area.height + 3,
                            },
                            None => {
                                let text = Text::from(format!("⚠ could not render {}", media.label));
                                let height = required_size(&text, content_width).1 + 1;
                                Plan::Note {
                                    text,
                                    style: error_style,
                                    height,
                                }
                            }
                        }
                    }
                    BlockImage::Ready { .. } => {
                        let text =
                            Text::from(format!("▣ {} (terminal graphics unavailable)", media.label));
                        let height = required_size(&text, content_width).1 + 1;
                        Plan::Note {
                            text,
                            style: muted_style,
                            height,
                        }
                    }
                },
            };
            total += u32::from(plan.height());
            plans.push(plan);
        }

        // Clamp scroll against the real content height.
        let max_scroll = total.saturating_sub(u32::from(viewport_height)) as u16;
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        let win_top = u32::from(self.scroll);
        let win_bottom = win_top + u32::from(viewport_height);

        // Draw visible blocks.
        let mut top: u32 = 0;
        for plan in &plans {
            let height = u32::from(plan.height());
            let bottom = top + height;
            if bottom <= win_top {
                top = bottom;
                continue;
            }
            if top >= win_bottom {
                break;
            }

            match plan {
                Plan::Text { index, .. } => {
                    let text = texts[*index].as_ref().expect("markdown block has text");
                    draw_paragraph(surface, content, top, win_top, win_bottom, text, None);
                }
                Plan::Note { text, style, .. } => {
                    draw_paragraph(surface, content, top, win_top, win_bottom, text, Some(*style));
                }
                Plan::Image {
                    index,
                    img_cols,
                    img_rows,
                    label,
                    ..
                } => {
                    let id = IMAGE_ID_BASE + *index as u32;
                    // The image occupies content rows [top+1, top+1+img_rows); one row of top
                    // padding separates it from the preceding block. The caption sits on the row
                    // just below the image. Only rows intersecting the viewport are drawn, so a
                    // partially-scrolled (or very tall) diagram still renders correctly.
                    let img_top = top + 1;
                    let img_bottom = img_top + u32::from(*img_rows);
                    let vis_start = img_top.max(win_top);
                    let vis_end = img_bottom.min(win_bottom);

                    if vis_end > vis_start && *img_cols > 0 {
                        let image_row_start = (vis_start - img_top) as u16;
                        let n_rows = (vis_end - vis_start) as u16;
                        let screen_y = content.y + (vis_start - win_top) as u16;
                        let x0 = content.x + content.width.saturating_sub(*img_cols) / 2;

                        // Display half: write the visible placeholder rows into the text grid.
                        kitty_graphics::place_image_rows(
                            surface,
                            x0,
                            screen_y,
                            *img_cols,
                            image_row_start,
                            n_rows,
                            id,
                        );

                        // Transmit half: (re-)transmit the image + virtual placement. Keyed on
                        // content (not position), so scrolling within view does not re-transmit.
                        if let Some(preview) = get_scaled(
                            cache,
                            blocks,
                            *index,
                            content_width,
                            max_img_rows,
                            cell_w,
                            cell_h,
                        ) {
                            ctx.media.push(MediaCommand::Image(MediaImage {
                                id,
                                area: Rect::new(0, 0, *img_cols, *img_rows),
                                width: preview.width,
                                height: preview.height,
                                payload_hash: preview.payload_hash,
                                png: preview.png.clone(),
                            }));
                        }
                    }

                    let caption_row = img_bottom;
                    if caption_row >= win_top && caption_row < win_bottom {
                        let screen_y = content.y + (caption_row - win_top) as u16;
                        draw_centered(surface, content, screen_y, label, caption_style);
                    }
                }
            }

            top = bottom;
        }
    }

    fn cursor(&self, _area: Rect, _editor: &helix_view::Editor) -> (Option<helix_core::Position>, CursorKind) {
        (None, CursorKind::Hidden)
    }

    fn id(&self) -> Option<&'static str> {
        Some(ID)
    }
}

/// Render a (possibly vertically-clipped) paragraph block into the content column.
fn draw_paragraph(
    surface: &mut Surface,
    content: Rect,
    top: u32,
    win_top: u32,
    win_bottom: u32,
    text: &Text,
    style: Option<Style>,
) {
    let bottom = top + text_height_hint(text, content.width);
    // Lines of this block scrolled above the viewport.
    let skip = win_top.saturating_sub(top) as u16;
    let screen_y = content.y + top.saturating_sub(win_top) as u16;
    let visible = (win_bottom.min(bottom) - top.max(win_top)) as u16;
    if visible == 0 {
        return;
    }
    let area = Rect::new(content.x, screen_y, content.width, visible);
    let mut par = Paragraph::new(text).wrap(Wrap { trim: false }).scroll((skip, 0));
    if let Some(style) = style {
        par = par.style(style);
    }
    par.render(area, surface);
}

fn text_height_hint(text: &Text, width: u16) -> u32 {
    u32::from(required_size(text, width).1)
}

/// Draw a single centered line of text within `area`, clipped to its width.
fn draw_centered(surface: &mut Surface, area: Rect, y: u16, message: &str, style: Style) {
    if y < area.y || y >= area.y + area.height {
        return;
    }
    let width = area.width as usize;
    let len = message.chars().count().min(width);
    let x = area.x + ((width - len) / 2) as u16;
    surface.set_stringn(x, y, message, width, style);
}

/// Fetch (and cache) the scaled image for a media block at the requested cell size.
fn get_scaled<'c>(
    cache: &'c mut HashMap<(usize, u16, u16), Option<ImagePreview>>,
    blocks: &[PreviewBlock],
    index: usize,
    cols: u16,
    rows: u16,
    cell_w: u32,
    cell_h: u32,
) -> Option<&'c ImagePreview> {
    let entry = cache.entry((index, cols, rows)).or_insert_with(|| {
        let PreviewBlock::Media(media) = &blocks[index] else {
            return None;
        };
        let BlockImage::Ready { bytes, .. } = &media.content else {
            return None;
        };
        let cell_area = Rect::new(0, 0, cols, rows);
        decode_image_preview(Path::new(""), bytes, cell_area, cell_w, cell_h).ok()
    });
    entry.as_ref()
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

enum SpecialKind {
    Mermaid(String),
    Image { url: String, alt: String },
}

struct Special {
    range: std::ops::Range<usize>,
    kind: SpecialKind,
}

/// Split `contents` into an ordered list of blocks. All mermaid diagram renders are kicked off in
/// parallel threads immediately, then image files are loaded, and finally the thread results are
/// joined in order to assemble the final block list.
fn parse_blocks(
    contents: &str,
    base_dir: Option<&Path>,
    renderer: &MermaidRenderer,
    config_loader: &Arc<ArcSwap<syntax::Loader>>,
) -> Vec<PreviewBlock> {
    let specials = find_special_ranges(contents);

    // Spawn one thread per mermaid diagram so all renders run concurrently.
    // Image specials get None; they're fast file reads done inline below.
    let handles: Vec<Option<std::thread::JoinHandle<BlockImage>>> = specials
        .iter()
        .map(|s| match &s.kind {
            SpecialKind::Mermaid(source) => {
                let renderer = renderer.clone();
                let source = source.clone();
                Some(std::thread::spawn(move || {
                    match renderer.render_png(&source) {
                        Ok(bytes) => BlockImage::from_bytes(bytes),
                        Err(err) => BlockImage::Error(err.to_string()),
                    }
                }))
            }
            SpecialKind::Image { .. } => None,
        })
        .collect();

    let mut blocks = Vec::new();
    let mut cursor = 0usize;
    let push_text = |blocks: &mut Vec<PreviewBlock>, text: &str| {
        if !text.trim().is_empty() {
            blocks.push(PreviewBlock::Markdown(Markdown::new(
                text.to_string(),
                config_loader.clone(),
            )));
        }
    };

    for (special, handle) in specials.into_iter().zip(handles) {
        if special.range.start > cursor {
            push_text(&mut blocks, &contents[cursor..special.range.start]);
        }
        let block = match (special.kind, handle) {
            (SpecialKind::Mermaid(_), Some(handle)) => {
                let content = handle
                    .join()
                    .unwrap_or_else(|_| BlockImage::Error("render thread panicked".to_string()));
                PreviewBlock::Media(MediaBlock {
                    label: "mermaid diagram".to_string(),
                    content,
                })
            }
            (SpecialKind::Image { url, alt }, None) => build_image_block(url, alt, base_dir),
            _ => unreachable!(),
        };
        blocks.push(block);
        cursor = special.range.end;
    }
    if cursor < contents.len() {
        push_text(&mut blocks, &contents[cursor..]);
    }

    blocks
}

fn build_image_block(url: String, alt: String, base_dir: Option<&Path>) -> PreviewBlock {
    let content = if url.starts_with("http://") || url.starts_with("https://") {
        BlockImage::Error(format!("remote images are not supported: {url}"))
    } else {
        let path = resolve_path(&url, base_dir);
        match std::fs::read(&path) {
            Ok(bytes) => BlockImage::from_bytes(bytes),
            Err(err) => BlockImage::Error(format!("could not read {}: {err}", path.display())),
        }
    };
    let label = if alt.trim().is_empty() { url } else { alt };
    PreviewBlock::Media(MediaBlock { label, content })
}

fn resolve_path(url: &str, base_dir: Option<&Path>) -> PathBuf {
    let path = Path::new(url);
    if path.is_absolute() {
        path.to_path_buf()
    } else if let Some(base) = base_dir {
        base.join(path)
    } else {
        path.to_path_buf()
    }
}

/// Locate the byte ranges of block-level special content: fenced ```mermaid blocks and paragraphs
/// that consist solely of a single image. Returned ranges are sorted and non-overlapping.
fn find_special_ranges(contents: &str) -> Vec<Special> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(contents, options).into_offset_iter();

    let mut specials = Vec::new();

    // Active mermaid code block: (start byte, accumulated source).
    let mut mermaid: Option<(usize, String)> = None;

    // Active paragraph, tracked to detect "standalone image" paragraphs.
    struct ParaState {
        start: usize,
        image_url: Option<String>,
        image_alt: String,
        in_image: bool,
        image_count: u32,
        stray_text: bool,
    }
    let mut para: Option<ParaState> = None;

    for (event, range) in parser {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang)))
                if lang.trim().eq_ignore_ascii_case("mermaid") =>
            {
                mermaid = Some((range.start, String::new()));
            }
            Event::Text(text) if mermaid.is_some() => {
                if let Some((_, source)) = mermaid.as_mut() {
                    source.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((start, source)) = mermaid.take() {
                    specials.push(Special {
                        range: start..range.end,
                        kind: SpecialKind::Mermaid(source),
                    });
                }
            }
            Event::Start(Tag::Paragraph) => {
                para = Some(ParaState {
                    start: range.start,
                    image_url: None,
                    image_alt: String::new(),
                    in_image: false,
                    image_count: 0,
                    stray_text: false,
                });
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                if let Some(state) = para.as_mut() {
                    state.in_image = true;
                    state.image_count += 1;
                    state.image_url = Some(dest_url.to_string());
                }
            }
            Event::End(TagEnd::Image) => {
                if let Some(state) = para.as_mut() {
                    state.in_image = false;
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some(state) = para.as_mut() {
                    if state.in_image {
                        state.image_alt.push_str(&text);
                    } else if !text.trim().is_empty() {
                        state.stray_text = true;
                    }
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if let Some(state) = para.take() {
                    if state.image_count == 1 && !state.stray_text {
                        if let Some(url) = state.image_url {
                            specials.push(Special {
                                range: state.start..range.end,
                                kind: SpecialKind::Image {
                                    url,
                                    alt: state.image_alt,
                                },
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }

    specials.sort_by_key(|special| special.range.start);
    // Drop any overlapping ranges, keeping the earliest.
    let mut result: Vec<Special> = Vec::new();
    for special in specials {
        if result
            .last()
            .is_none_or(|last| special.range.start >= last.range.end)
        {
            result.push(special);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loader() -> Arc<ArcSwap<syntax::Loader>> {
        let config = helix_loader::config::default_lang_config();
        Arc::new(ArcSwap::from_pointee(
            syntax::Loader::new(config.try_into().unwrap()).unwrap(),
        ))
    }

    fn missing_renderer() -> MermaidRenderer {
        let mut config = helix_view::editor::MarkdownPreviewConfig::default();
        config.diagram_renderer = "definitely-not-a-real-binary-xyz".to_string();
        MermaidRenderer::from_config(&config)
    }

    #[test]
    fn detects_mermaid_block() {
        let src = "# Title\n\nText before.\n\n```mermaid\ngraph TD;\n A-->B;\n```\n\nAfter.\n";
        let specials = find_special_ranges(src);
        assert_eq!(specials.len(), 1);
        match &specials[0].kind {
            SpecialKind::Mermaid(source) => {
                assert!(source.contains("graph TD"));
                assert!(source.contains("A-->B"));
            }
            _ => panic!("expected mermaid"),
        }
        // The detected range should slice out the fenced block.
        let slice = &src[specials[0].range.clone()];
        assert!(slice.contains("```mermaid"));
    }

    #[test]
    fn ignores_non_mermaid_code_block() {
        let src = "```rust\nfn main() {}\n```\n";
        assert!(find_special_ranges(src).is_empty());
    }

    #[test]
    fn detects_standalone_image_but_not_inline() {
        let standalone = "![a diagram](diagram.png)\n";
        let specials = find_special_ranges(standalone);
        assert_eq!(specials.len(), 1);
        match &specials[0].kind {
            SpecialKind::Image { url, alt } => {
                assert_eq!(url, "diagram.png");
                assert_eq!(alt, "a diagram");
            }
            _ => panic!("expected image"),
        }

        let inline = "Here is an image ![x](y.png) within a sentence.\n";
        assert!(
            find_special_ranges(inline).is_empty(),
            "inline images should stay part of the text"
        );
    }

    #[test]
    fn splits_into_text_and_media_blocks() {
        let src = "# Heading\n\nProse.\n\n```mermaid\ngraph TD; A-->B;\n```\n\nMore prose.\n";
        let blocks = parse_blocks(src, None, &missing_renderer(), &loader());
        // text, media (mermaid -> error because renderer missing), text
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0], PreviewBlock::Markdown(_)));
        assert!(matches!(blocks[2], PreviewBlock::Markdown(_)));
        match &blocks[1] {
            PreviewBlock::Media(media) => {
                assert_eq!(media.label, "mermaid diagram");
                assert!(matches!(media.content, BlockImage::Error(_)));
            }
            _ => panic!("expected media block"),
        }
    }

}
