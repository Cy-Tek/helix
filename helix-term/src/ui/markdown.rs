use crate::compositor::{Component, Context};
use arc_swap::ArcSwap;
use tui::{
    buffer::Buffer as Surface,
    text::{Span, Spans, Text},
};

use std::sync::Arc;

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use helix_core::{
    syntax::{self, HighlightEvent, OverlayHighlights},
    RopeSlice, Syntax,
};
use helix_view::{
    graphics::{Margin, Rect, Style},
    theme::Modifier,
    Theme,
};

fn styled_multiline_text<'a>(text: &str, style: Style) -> Text<'a> {
    let spans: Vec<_> = text
        .lines()
        .map(|line| Span::styled(line.to_string(), style))
        .map(Spans::from)
        .collect();
    Text::from(spans)
}

pub fn highlighted_code_block<'a>(
    text: &str,
    language: &str,
    theme: Option<&Theme>,
    loader: &syntax::Loader,
    // Optional overlay highlights to mix in with the syntax highlights.
    //
    // Note that `OverlayHighlights` is typically used with char indexing but the only caller
    // which passes this parameter currently passes **byte indices** instead.
    additional_highlight_spans: Option<OverlayHighlights>,
) -> Text<'a> {
    let mut spans = Vec::new();
    let mut lines = Vec::new();

    let get_theme = |key: &str| -> Style { theme.map(|t| t.get(key)).unwrap_or_default() };
    let text_style = get_theme(Markdown::TEXT_STYLE);
    let code_style = get_theme(Markdown::BLOCK_STYLE);

    let theme = match theme {
        Some(t) => t,
        None => return styled_multiline_text(text, code_style),
    };

    let ropeslice = RopeSlice::from(text);
    let Some(syntax) = loader
        .language_for_match(RopeSlice::from(language))
        .and_then(|lang| Syntax::new(ropeslice, lang, loader).ok())
    else {
        return styled_multiline_text(text, code_style);
    };

    let mut syntax_highlighter = syntax.highlighter(ropeslice, loader, ..);
    let mut syntax_highlight_stack = Vec::new();
    let mut overlay_highlight_stack = Vec::new();
    let mut overlay_highlighter = syntax::OverlayHighlighter::new(additional_highlight_spans);
    let mut pos = 0;

    while pos < ropeslice.len_bytes() as u32 {
        if pos == syntax_highlighter.next_event_offset() {
            let (event, new_highlights) = syntax_highlighter.advance();
            if event == HighlightEvent::Refresh {
                syntax_highlight_stack.clear();
            }
            syntax_highlight_stack.extend(new_highlights);
        } else if pos == overlay_highlighter.next_event_offset() as u32 {
            let (event, new_highlights) = overlay_highlighter.advance();
            if event == HighlightEvent::Refresh {
                overlay_highlight_stack.clear();
            }
            overlay_highlight_stack.extend(new_highlights)
        }

        let start = pos;
        pos = syntax_highlighter
            .next_event_offset()
            .min(overlay_highlighter.next_event_offset() as u32);
        if pos == u32::MAX {
            pos = ropeslice.len_bytes() as u32;
        }
        if pos == start {
            continue;
        }
        // The highlighter should always move forward.
        // If the highlighter malfunctions, bail on syntax highlighting and log an error.
        debug_assert!(pos > start);
        if pos < start {
            log::error!("Failed to highlight '{language}': {text:?}");
            return styled_multiline_text(text, code_style);
        }

        let style = syntax_highlight_stack
            .iter()
            .chain(overlay_highlight_stack.iter())
            .fold(text_style, |acc, highlight| {
                acc.patch(theme.highlight(*highlight))
            });

        let mut slice = &text[start as usize..pos as usize];
        // TODO: do we need to handle all unicode line endings
        // here, or is just '\n' okay?
        while let Some(end) = slice.find('\n') {
            // emit span up to newline
            let text = &slice[..end];
            let text = text.replace('\t', "    "); // replace tabs
            let span = Span::styled(text, style);
            spans.push(span);

            // truncate slice to after newline
            slice = &slice[end + 1..];

            // make a new line
            let spans = std::mem::take(&mut spans);
            lines.push(Spans::from(spans));
        }

        if !slice.is_empty() {
            let span = Span::styled(slice.replace('\t', "    "), style);
            spans.push(span);
        }
    }

    if !spans.is_empty() {
        let spans = std::mem::take(&mut spans);
        lines.push(Spans::from(spans));
    }

    Text::from(lines)
}

pub struct Markdown {
    contents: String,

    config_loader: Arc<ArcSwap<syntax::Loader>>,
}

// TODO: pre-render and self reference via Pin
// better yet, just use Tendril + subtendril for references

impl Markdown {
    const TEXT_STYLE: &'static str = "ui.text";
    const BLOCK_STYLE: &'static str = "markup.raw.inline";
    const RULE_STYLE: &'static str = "punctuation.special";
    const UNNUMBERED_LIST_STYLE: &'static str = "markup.list.unnumbered";
    const NUMBERED_LIST_STYLE: &'static str = "markup.list.numbered";
    const HEADING_STYLES: [&'static str; 6] = [
        "markup.heading.1",
        "markup.heading.2",
        "markup.heading.3",
        "markup.heading.4",
        "markup.heading.5",
        "markup.heading.6",
    ];
    const INDENT: &'static str = "  ";

    pub fn new(contents: String, config_loader: Arc<ArcSwap<syntax::Loader>>) -> Self {
        Self {
            contents,
            config_loader,
        }
    }

    pub fn parse(&self, theme: Option<&Theme>, max_table_width: Option<u16>) -> tui::text::Text<'_> {
        fn push_line<'a>(spans: &mut Vec<Span<'a>>, lines: &mut Vec<Spans<'a>>) {
            let spans = std::mem::take(spans);
            if !spans.is_empty() {
                lines.push(Spans::from(spans));
            }
        }

        let mut options = Options::empty();
        options.insert(Options::ENABLE_STRIKETHROUGH);
        options.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(&self.contents, options);

        // TODO: if possible, render links as terminal hyperlinks: https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda
        let mut tags = Vec::new();
        let mut spans = Vec::new();
        let mut lines = Vec::new();
        let mut list_stack = Vec::new();

        let get_indent = |level: usize| {
            if level < 1 {
                String::new()
            } else {
                Self::INDENT.repeat(level - 1)
            }
        };

        let get_theme = |key: &str| -> Style { theme.map(|t| t.get(key)).unwrap_or_default() };
        let text_style = get_theme(Self::TEXT_STYLE);
        let code_style = get_theme(Self::BLOCK_STYLE);
        let numbered_list_style = get_theme(Self::NUMBERED_LIST_STYLE);
        let unnumbered_list_style = get_theme(Self::UNNUMBERED_LIST_STYLE);
        let rule_style = get_theme(Self::RULE_STYLE);
        let heading_styles: Vec<Style> = Self::HEADING_STYLES
            .iter()
            .map(|key| get_theme(key))
            .collect();

        // Transform text in `<code>` blocks into `Event::Code`
        let mut in_code = false;
        let parser = parser.filter_map(|event| match event {
            Event::Html(tag)
                if tag.starts_with("<code") && matches!(tag.chars().nth(5), Some(' ' | '>')) =>
            {
                in_code = true;
                None
            }
            Event::Html(tag) if *tag == *"</code>" => {
                in_code = false;
                None
            }
            Event::Text(text) if in_code => Some(Event::Code(text)),
            _ => Some(event),
        });

        // Table accumulation state.
        let mut in_table = false;
        let mut tbl_in_head = false;
        let mut tbl_in_cell = false;
        let mut tbl_header: Vec<String> = Vec::new();
        let mut tbl_body: Vec<Vec<String>> = Vec::new();
        let mut tbl_cur_row: Vec<String> = Vec::new();
        let mut tbl_cur_cell = String::new();

        for event in parser {
            match event {
                Event::Start(Tag::List(list)) => {
                    // if the list stack is not empty this is a sub list, in that
                    // case we need to push the current line before proceeding
                    if !list_stack.is_empty() {
                        push_line(&mut spans, &mut lines);
                    }

                    list_stack.push(list);
                }
                Event::End(TagEnd::List(_)) => {
                    list_stack.pop();

                    // whenever top-level list closes, empty line
                    if list_stack.is_empty() {
                        lines.push(Spans::default());
                    }
                }
                Event::Start(Tag::Item) => {
                    if list_stack.is_empty() {
                        log::warn!("markdown parsing error, list item without list");
                    }

                    tags.push(Tag::Item);

                    // get the appropriate bullet for the current list
                    let (bullet, bullet_style) = list_stack
                        .last()
                        .unwrap_or(&None) // use the '- ' bullet in case the list stack would be empty
                        .map_or((String::from("• "), unnumbered_list_style), |number| {
                            (format!("{}. ", number), numbered_list_style)
                        });

                    // increment the current list number if there is one
                    if let Some(v) = list_stack.last_mut().unwrap_or(&mut None).as_mut() {
                        *v += 1;
                    }

                    let prefix = get_indent(list_stack.len()) + bullet.as_str();
                    spans.push(Span::styled(prefix, bullet_style));
                }
                Event::Start(Tag::Table(_)) => {
                    in_table = true;
                    tbl_header.clear();
                    tbl_body.clear();
                }
                Event::Start(Tag::TableHead) => {
                    tbl_in_head = true;
                }
                Event::Start(Tag::TableRow) => {
                    tbl_cur_row.clear();
                }
                Event::Start(Tag::TableCell) => {
                    tbl_cur_cell.clear();
                    tbl_in_cell = true;
                }
                Event::Start(tag) => {
                    tags.push(tag);
                    if spans.is_empty() && !list_stack.is_empty() {
                        // TODO: could push indent + 2 or 3 spaces to align with
                        // the rest of the list.
                        spans.push(Span::from(get_indent(list_stack.len())));
                    }
                }
                Event::End(TagEnd::TableCell) => {
                    tbl_in_cell = false;
                    tbl_cur_row.push(std::mem::take(&mut tbl_cur_cell).trim().to_string());
                }
                Event::End(TagEnd::TableRow) => {
                    let row = std::mem::take(&mut tbl_cur_row);
                    if tbl_in_head {
                        tbl_header = row;
                    } else {
                        tbl_body.push(row);
                    }
                }
                Event::End(TagEnd::TableHead) => {
                    // TableHead contains TableCell events directly (no TableRow wrapper),
                    // so flush whatever accumulated in tbl_cur_row into tbl_header here.
                    if !tbl_cur_row.is_empty() {
                        tbl_header = std::mem::take(&mut tbl_cur_row);
                    }
                    tbl_in_head = false;
                }
                Event::End(TagEnd::Table) => {
                    in_table = false;
                    if !tbl_header.is_empty() {
                        let max_w = max_table_width.map(|w| w as usize).unwrap_or(100);
                        let rendered = render_table_as_text(
                            &tbl_header,
                            &tbl_body,
                            max_w,
                            text_style.add_modifier(Modifier::BOLD),
                            get_theme(Self::RULE_STYLE),
                            text_style,
                        );
                        lines.extend(rendered);
                    }
                }
                Event::End(tag) => {
                    tags.pop();
                    match tag {
                        TagEnd::Heading(_)
                        | TagEnd::Paragraph
                        | TagEnd::CodeBlock
                        | TagEnd::Item => {
                            push_line(&mut spans, &mut lines);
                        }
                        _ => (),
                    }

                    // whenever heading, code block or paragraph closes, empty line
                    match tag {
                        TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::CodeBlock => {
                            lines.push(Spans::default());
                        }
                        _ => (),
                    }
                }
                Event::Text(text) => {
                    if in_table && tbl_in_cell {
                        tbl_cur_cell.push_str(&text);
                    } else if let Some(Tag::CodeBlock(kind)) = tags.last() {
                        let language = match kind {
                            CodeBlockKind::Fenced(language) => language,
                            CodeBlockKind::Indented => "",
                        };
                        let tui_text = highlighted_code_block(
                            &text,
                            language,
                            theme,
                            &self.config_loader.load(),
                            None,
                        );
                        lines.extend(tui_text.lines);
                    } else {
                        let style = match tags.last() {
                            Some(Tag::Heading { level, .. }) => match level {
                                HeadingLevel::H1 => heading_styles[0],
                                HeadingLevel::H2 => heading_styles[1],
                                HeadingLevel::H3 => heading_styles[2],
                                HeadingLevel::H4 => heading_styles[3],
                                HeadingLevel::H5 => heading_styles[4],
                                HeadingLevel::H6 => heading_styles[5],
                            },
                            Some(Tag::Emphasis) => text_style.add_modifier(Modifier::ITALIC),
                            Some(Tag::Strong) => text_style.add_modifier(Modifier::BOLD),
                            Some(Tag::Strikethrough) => {
                                text_style.add_modifier(Modifier::CROSSED_OUT)
                            }
                            _ => text_style,
                        };
                        spans.push(Span::styled(text, style));
                    }
                }
                Event::Code(text) if in_table && tbl_in_cell => {
                    tbl_cur_cell.push_str(&text);
                }
                Event::Code(text) | Event::Html(text) => {
                    spans.push(Span::styled(text, code_style));
                }
                Event::SoftBreak | Event::HardBreak if in_table && tbl_in_cell => {
                    tbl_cur_cell.push(' ');
                }
                Event::SoftBreak | Event::HardBreak => {
                    push_line(&mut spans, &mut lines);
                    if !list_stack.is_empty() {
                        // TODO: could push indent + 2 or 3 spaces to align with
                        // the rest of the list.
                        spans.push(Span::from(get_indent(list_stack.len())));
                    }
                }
                Event::Rule => {
                    lines.push(Spans::from(Span::styled("───", rule_style)));
                    lines.push(Spans::default());
                }
                // TaskListMarker(bool) true if checked
                _ => {
                    log::warn!("unhandled markdown event {:?}", event);
                }
            }
            // build up a vec of Paragraph tui widgets
        }

        if !spans.is_empty() {
            lines.push(Spans::from(spans));
        }

        // if last line is empty, remove it
        if let Some(line) = lines.last() {
            if line.0.is_empty() {
                lines.pop();
            }
        }

        Text::from(lines)
    }
}

/// Render a parsed markdown table as a flat list of styled lines.
///
/// Each cell is word-wrapped to its column width. Columns are naturally sized to
/// their widest content; if the total exceeds `max_width` they are proportionally
/// scaled down (the content then wraps within the narrower column rather than
/// being truncated).
fn render_table_as_text(
    header: &[String],
    body: &[Vec<String>],
    max_width: usize,
    header_style: Style,
    separator_style: Style,
    text_style: Style,
) -> Vec<Spans<'static>> {
    let n_cols = header.len();
    let sep_total = n_cols.saturating_sub(1) * 2; // 2-space column gaps

    // Natural column widths derived from header + body cells.
    let mut col_widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in body {
        for (i, cell) in row.iter().enumerate() {
            if i < n_cols {
                col_widths[i] = col_widths[i].max(cell.chars().count());
            }
        }
    }

    // Proportionally shrink columns when the total would exceed max_width.
    let total: usize = col_widths.iter().sum::<usize>() + sep_total;
    if total > max_width {
        let available = max_width.saturating_sub(sep_total);
        let natural_sum: usize = col_widths.iter().sum();
        col_widths = col_widths
            .iter()
            .map(|&w| ((w * available) / natural_sum.max(1)).max(3))
            .collect();
    }

    let mut result: Vec<Spans<'static>> = Vec::with_capacity(body.len() * 2 + 3);

    // Header row (bold, wrapped).
    result.extend(table_row_wrapped(header, &col_widths, n_cols, header_style));

    // Separator line.
    let sep_spans: Vec<Span<'static>> = (0..n_cols)
        .flat_map(|i| {
            let mut v: Vec<Span<'static>> =
                vec![Span::styled("─".repeat(col_widths[i]), separator_style)];
            if i + 1 < n_cols {
                v.push(Span::raw("  "));
            }
            v
        })
        .collect();
    result.push(Spans::from(sep_spans));

    // Body rows (wrapped).
    for row in body {
        result.extend(table_row_wrapped(row, &col_widths, n_cols, text_style));
    }

    // Trailing blank line (matches the spacing after headings / paragraphs).
    result.push(Spans::default());
    result
}

/// Build one or more `Spans` lines for a table row by word-wrapping each cell to
/// its column width. All cells in the same logical row are padded to the same
/// number of lines so columns stay vertically aligned.
fn table_row_wrapped(
    cells: &[String],
    widths: &[usize],
    n_cols: usize,
    style: Style,
) -> Vec<Spans<'static>> {
    let wrapped: Vec<Vec<String>> = (0..n_cols)
        .map(|i| wrap_cell(cells.get(i).map(String::as_str).unwrap_or(""), widths[i]))
        .collect();

    let n_lines = wrapped.iter().map(|c| c.len()).max().unwrap_or(1);

    (0..n_lines)
        .map(|li| {
            let spans: Vec<Span<'static>> = (0..n_cols)
                .flat_map(|i| {
                    let text = wrapped[i].get(li).map(String::as_str).unwrap_or("");
                    let padded = format!("{text:<width$}", width = widths[i]);
                    let mut v: Vec<Span<'static>> = vec![Span::styled(padded, style)];
                    if i + 1 < n_cols {
                        v.push(Span::raw("  "));
                    }
                    v
                })
                .collect();
            Spans::from(spans)
        })
        .collect()
}

/// Word-wrap `text` to fit within `width` characters per line, returning one
/// string per line. Words longer than `width` are hard-broken at the boundary.
fn wrap_cell(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len: usize = 0;

    for word in text.split_whitespace() {
        let word_len = word.chars().count();

        if word_len > width {
            // Hard-break: flush any in-progress line first, then slice the word.
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_len = 0;
            }
            let chars: Vec<char> = word.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                let end = (i + width).min(chars.len());
                let chunk: String = chars[i..end].iter().collect();
                if end < chars.len() {
                    lines.push(chunk);
                } else {
                    current = chunk;
                    current_len = end - i;
                }
                i = end;
            }
        } else if current.is_empty() {
            current.push_str(word);
            current_len = word_len;
        } else if current_len + 1 + word_len <= width {
            current.push(' ');
            current.push_str(word);
            current_len += 1 + word_len;
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_len = word_len;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

impl Component for Markdown {
    fn render(&mut self, area: Rect, surface: &mut Surface, cx: &mut Context) {
        use tui::widgets::{Paragraph, Widget, Wrap};

        let render_width = area.width.saturating_sub(2);
        let text = self.parse(Some(&cx.editor.theme), Some(render_width));

        let par = Paragraph::new(&text)
            .wrap(Wrap { trim: false })
            .scroll((cx.scroll.unwrap_or_default() as u16, 0));

        let margin = Margin::all(1);
        par.render(area.inner(margin), surface);
    }

    fn required_size(&mut self, viewport: (u16, u16)) -> Option<(u16, u16)> {
        let padding = 2;
        // TODO: account for tab width
        let max_text_width = (viewport.0.saturating_sub(padding)).min(120);
        let contents = self.parse(None, Some(max_text_width));
        let (width, height) = crate::ui::text::required_size(&contents, max_text_width);

        Some((width + padding, height + padding))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_swap::ArcSwap;
    use std::sync::Arc;

    fn loader() -> Arc<ArcSwap<helix_core::syntax::Loader>> {
        let config = helix_loader::config::default_lang_config();
        Arc::new(ArcSwap::from_pointee(
            helix_core::syntax::Loader::new(config.try_into().unwrap()).unwrap(),
        ))
    }

    #[test]
    fn table_produces_lines() {
        let md = Markdown::new(
            "| A | B |\n| - | - |\n| x | y |\n".to_string(),
            loader(),
        );
        let text = md.parse(None, None);
        let all_text: String = text
            .lines
            .iter()
            .map(|s| s.0.iter().map(|sp| sp.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.lines.len() >= 3,
            "expected ≥3 lines from table, got {}; content:\n{}",
            text.lines.len(),
            all_text
        );
    }
}
