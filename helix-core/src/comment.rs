//! This module contains the functionality toggle comments on lines over the selection
//! using the comment character defined in the user's `languages.toml`

use smallvec::SmallVec;

use crate::{
    syntax::config::BlockCommentToken, Change, Range, Rope, RopeSlice, Selection, Tendril,
    Transaction,
};
use helix_stdx::rope::RopeSliceExt;
use std::borrow::Cow;
use std::str::FromStr;

pub const DEFAULT_COMMENT_TOKEN: &str = "#";
pub const DEFAULT_COMMENT_BOX_WIDTH: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentBoxAlignment {
    Left,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentBoxStyle {
    Plain,
    Subheading,
    Heading,
    Ruler,
    Slash,
    Star,
    Hash,
    Box,
    Sidebar,
    Fold,
    DocHeading,
}

impl FromStr for CommentBoxStyle {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "plain" | "label" => Ok(Self::Plain),
            "subheading" | "sub" | "minor" | "dash" | "dashed" => Ok(Self::Subheading),
            "heading" | "major" | "equals" => Ok(Self::Heading),
            "ruler" | "centered" | "centered-ruler" => Ok(Self::Ruler),
            "slash" | "slashes" | "banner" => Ok(Self::Slash),
            "star" | "stars" => Ok(Self::Star),
            "hash" | "markdown" | "md" => Ok(Self::Hash),
            "box" | "boxed" => Ok(Self::Box),
            "sidebar" | "side-bar" | "bar" => Ok(Self::Sidebar),
            "fold" | "region" => Ok(Self::Fold),
            "doc-heading" | "doc" | "docheading" => Ok(Self::DocHeading),
            _ => Err("unknown comment box style"),
        }
    }
}

fn comment_prefix(token: &str) -> String {
    format!("{token} ")
}

fn line_width(width: usize, prefix: &str) -> usize {
    width.saturating_sub(prefix.chars().count())
}

fn repeat_to_width(ch: char, width: usize) -> String {
    std::iter::repeat_n(ch, width).collect()
}

fn align_text(text: &str, width: usize, alignment: CommentBoxAlignment) -> String {
    let text_width = text.chars().count();
    if text_width >= width {
        return text.to_string();
    }

    let padding = width - text_width;
    match alignment {
        CommentBoxAlignment::Left => format!("{text}{}", " ".repeat(padding)),
        CommentBoxAlignment::Center => {
            let left = padding / 2;
            let right = padding - left;
            format!("{}{}{}", " ".repeat(left), text, " ".repeat(right))
        }
    }
}

fn fill_around(text: &str, width: usize, fill: char, alignment: CommentBoxAlignment) -> String {
    let label = format!(" {text} ");
    let label_width = label.chars().count();
    if label_width >= width {
        return label;
    }

    let fill_width = width - label_width;
    let (left, right) = match alignment {
        CommentBoxAlignment::Left => (4.min(fill_width), fill_width.saturating_sub(4)),
        CommentBoxAlignment::Center => (fill_width / 2, fill_width - (fill_width / 2)),
    };

    format!(
        "{}{}{}",
        repeat_to_width(fill, left),
        label,
        repeat_to_width(fill, right)
    )
}

fn prefixed_line(prefix: &str, content: String, width: usize) -> String {
    let target = line_width(width, prefix);
    if content.chars().count() >= target {
        return format!("{prefix}{content}");
    }
    format!(
        "{prefix}{content}{}",
        " ".repeat(target - content.chars().count())
    )
}

fn doc_comment_prefix(token: &str) -> String {
    if token == "//" {
        "/// ".to_string()
    } else {
        comment_prefix(token)
    }
}

pub fn format_comment_box(
    token: &str,
    style: CommentBoxStyle,
    alignment: CommentBoxAlignment,
    width: usize,
    text_lines: &[String],
) -> String {
    let title = text_lines
        .first()
        .map(String::as_str)
        .unwrap_or_default()
        .trim();
    let subtitle_lines = text_lines.iter().skip(1).map(|line| line.trim());
    let prefix = comment_prefix(token);
    let content_width = line_width(width, &prefix);

    match style {
        CommentBoxStyle::Plain => {
            let mut lines = Vec::new();
            lines.push(prefixed_line(
                &prefix,
                align_text(title, content_width, alignment),
                width,
            ));
            lines.extend(subtitle_lines.map(|line| {
                prefixed_line(&prefix, align_text(line, content_width, alignment), width)
            }));
            lines.join("\n")
        }
        CommentBoxStyle::Subheading => prefixed_line(
            &prefix,
            fill_around(title, content_width, '-', alignment),
            width,
        ),
        CommentBoxStyle::Heading => {
            let rule = prefixed_line(&prefix, repeat_to_width('=', content_width), width);
            let title = prefixed_line(&prefix, align_text(title, content_width, alignment), width);
            format!("{rule}\n{title}\n{rule}")
        }
        CommentBoxStyle::Ruler => prefixed_line(
            &prefix,
            fill_around(title, content_width, '-', alignment),
            width,
        ),
        CommentBoxStyle::Slash => {
            let rule = format!("{}{}", token, repeat_to_width('/', width.saturating_sub(2)));
            let title = prefixed_line(&prefix, align_text(title, content_width, alignment), width);
            format!("{rule}\n{title}\n{rule}")
        }
        CommentBoxStyle::Star => {
            let rule = prefixed_line(&prefix, repeat_to_width('*', content_width), width);
            let title = prefixed_line(&prefix, align_text(title, content_width, alignment), width);
            format!("{rule}\n{title}\n{rule}")
        }
        CommentBoxStyle::Hash => prefixed_line(&prefix, format!("### {title}"), width),
        CommentBoxStyle::Box => {
            let border_width = content_width.saturating_sub(2);
            let inner_width = content_width.saturating_sub(4);
            let border = prefixed_line(
                &prefix,
                format!("+{}+", repeat_to_width('-', border_width)),
                width,
            );
            let mut lines = Vec::new();
            lines.push(border.clone());
            lines.push(prefixed_line(
                &prefix,
                format!("| {} |", fill_around(title, inner_width, '-', alignment)),
                width,
            ));
            lines.extend(subtitle_lines.map(|line| {
                prefixed_line(
                    &prefix,
                    format!("| {} |", align_text(line, inner_width, alignment)),
                    width,
                )
            }));
            lines.push(border);
            lines.join("\n")
        }
        CommentBoxStyle::Sidebar => {
            let mut lines = Vec::new();
            lines.push(prefixed_line(&prefix, format!("| {title}"), width));
            lines.extend(
                subtitle_lines.map(|line| prefixed_line(&prefix, format!("| {line}"), width)),
            );
            lines.join("\n")
        }
        CommentBoxStyle::Fold => {
            let end = if title.is_empty() {
                "endregion".to_string()
            } else {
                format!("endregion {title}")
            };
            format!(
                "{}\n{}",
                prefixed_line(&prefix, format!("region {title}"), width),
                prefixed_line(&prefix, end, width)
            )
        }
        CommentBoxStyle::DocHeading => {
            let prefix = doc_comment_prefix(token);
            let content_width = line_width(width, &prefix);
            prefixed_line(
                &prefix,
                format!("# {title}"),
                content_width + prefix.chars().count(),
            )
        }
    }
}

fn is_decoration_line(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    let without_box_edges = trimmed
        .strip_prefix('+')
        .and_then(|text| text.strip_suffix('+'))
        .unwrap_or(trimmed);

    without_box_edges
        .chars()
        .all(|ch| matches!(ch, '-' | '=' | '*' | '/' | '~' | '#'))
}

fn strip_comment_token<'a>(token: &str, line: &'a str) -> Option<&'a str> {
    let line = line.trim_start();
    let line = if token == "//" {
        line.strip_prefix("///")
            .or_else(|| line.strip_prefix(token))?
    } else {
        line.strip_prefix(token)?
    };
    Some(line.strip_prefix(' ').unwrap_or(line))
}

fn strip_box_edges(line: &str) -> &str {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line).trim();
    line.strip_suffix('|').unwrap_or(line).trim()
}

fn strip_filled_label(line: &str) -> &str {
    let line = line.trim();
    let Some(fill) = line.chars().next() else {
        return line;
    };
    if !matches!(fill, '-' | '=' | '*' | '/' | '~') {
        return line;
    }

    let left_width = line
        .chars()
        .take_while(|&ch| ch == fill)
        .map(char::len_utf8)
        .sum::<usize>();
    let after_left = &line[left_width..];
    if !after_left.starts_with(char::is_whitespace) {
        return line;
    }

    let title = after_left.trim_start();
    let end_without_fill = title.trim_end_matches(fill);
    if end_without_fill == title {
        return line;
    }

    end_without_fill.trim_end()
}

pub fn comment_box_text_lines(token: &str, text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = strip_comment_token(token, line)?;
            let line = line.trim();
            if is_decoration_line(line) {
                return None;
            }

            let line = strip_filled_label(strip_box_edges(line));
            let line = line
                .strip_prefix("region ")
                .or_else(|| line.strip_prefix("### "))
                .or_else(|| line.strip_prefix("# "))
                .unwrap_or(line)
                .trim();

            if line.starts_with("endregion") || line.is_empty() {
                None
            } else {
                Some(line.to_string())
            }
        })
        .collect()
}

/// Returns the longest matching comment token of the given line (if it exists).
pub fn get_comment_token<'a, S: AsRef<str>>(
    text: RopeSlice,
    tokens: &'a [S],
    line_num: usize,
) -> Option<&'a str> {
    let line = text.line(line_num);
    let start = line.first_non_whitespace_char()?;

    tokens
        .iter()
        .map(AsRef::as_ref)
        .filter(|token| line.slice(start..).starts_with(token))
        .max_by_key(|token| token.len())
}

/// Given text, a comment token, and a set of line indices, returns the following:
/// - Whether the given lines should be considered commented
///     - If any of the lines are uncommented, all lines are considered as such.
/// - The lines to change for toggling comments
///     - This is all provided lines excluding blanks lines.
/// - The column of the comment tokens
///     - Column of existing tokens, if the lines are commented; column to place tokens at otherwise.
/// - The margin to the right of the comment tokens
///     - Defaults to `1`. If any existing comment token is not followed by a space, changes to `0`.
fn find_line_comment(
    token: &str,
    text: RopeSlice,
    lines: impl IntoIterator<Item = usize>,
) -> (bool, Vec<usize>, usize, usize) {
    let mut commented = true;
    let mut to_change = Vec::new();
    let mut min = usize::MAX; // minimum col for first_non_whitespace_char
    let mut margin = 1;
    let token_len = token.chars().count();

    for line in lines {
        let line_slice = text.line(line);
        if let Some(pos) = line_slice.first_non_whitespace_char() {
            let len = line_slice.len_chars();

            min = std::cmp::min(min, pos);

            // line can be shorter than pos + token len
            let fragment = Cow::from(line_slice.slice(pos..std::cmp::min(pos + token.len(), len)));

            // as soon as one of the non-blank lines doesn't have a comment, the whole block is
            // considered uncommented.
            if fragment != token {
                commented = false;
            }

            // determine margin of 0 or 1 for uncommenting; if any comment token is not followed by a space,
            // a margin of 0 is used for all lines.
            if !matches!(line_slice.get_char(pos + token_len), Some(c) if c == ' ') {
                margin = 0;
            }

            // blank lines don't get pushed.
            to_change.push(line);
        }
    }

    (commented, to_change, min, margin)
}

#[must_use]
pub fn toggle_line_comments(doc: &Rope, selection: &Selection, token: Option<&str>) -> Transaction {
    let text = doc.slice(..);

    let token = token.unwrap_or(DEFAULT_COMMENT_TOKEN);
    let comment = Tendril::from(format!("{} ", token));

    let mut lines: Vec<usize> = Vec::with_capacity(selection.len());

    let mut min_next_line = 0;
    for selection in selection {
        let (start, end) = selection.line_range(text);
        let start = start.clamp(min_next_line, text.len_lines());
        let end = (end + 1).min(text.len_lines());

        lines.extend(start..end);
        min_next_line = end;
    }

    let (commented, to_change, min, margin) = find_line_comment(token, text, lines);

    let mut changes: Vec<Change> = Vec::with_capacity(to_change.len());

    for line in to_change {
        let pos = text.line_to_char(line) + min;

        if !commented {
            // comment line
            changes.push((pos, pos, Some(comment.clone())));
        } else {
            // uncomment line
            changes.push((pos, pos + token.len() + margin, None));
        }
    }

    Transaction::change(doc, changes.into_iter())
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommentChange {
    Commented {
        range: Range,
        start_pos: usize,
        end_pos: usize,
        start_margin: bool,
        end_margin: bool,
        start_token: String,
        end_token: String,
    },
    Uncommented {
        range: Range,
        start_pos: usize,
        end_pos: usize,
        start_token: String,
        end_token: String,
    },
    Whitespace {
        range: Range,
    },
}

pub fn find_block_comments(
    tokens: &[BlockCommentToken],
    text: RopeSlice,
    selection: &Selection,
) -> (bool, Vec<CommentChange>) {
    let mut commented = true;
    let mut only_whitespace = true;
    let mut comment_changes = Vec::with_capacity(selection.len());
    let default_tokens = tokens.first().cloned().unwrap_or_default();
    let mut start_token = default_tokens.start.clone();
    let mut end_token = default_tokens.end.clone();

    let mut tokens = tokens.to_vec();
    // sort the tokens by length, so longer tokens will match first
    tokens.sort_by(|a, b| {
        if a.start.len() == b.start.len() {
            b.end.len().cmp(&a.end.len())
        } else {
            b.start.len().cmp(&a.start.len())
        }
    });
    for range in selection {
        let selection_slice = range.slice(text);
        if let (Some(start_pos), Some(end_pos)) = (
            selection_slice.first_non_whitespace_char(),
            selection_slice.last_non_whitespace_char(),
        ) {
            let mut line_commented = false;
            let mut after_start = 0;
            let mut before_end = 0;
            let len = (end_pos + 1) - start_pos;

            for BlockCommentToken { start, end } in &tokens {
                let start_len = start.chars().count();
                let end_len = end.chars().count();
                after_start = start_pos + start_len;
                before_end = end_pos.saturating_sub(end_len);

                if len >= start_len + end_len {
                    let start_fragment = selection_slice.slice(start_pos..after_start);
                    let end_fragment = selection_slice.slice(before_end + 1..end_pos + 1);

                    // block commented with these tokens
                    if start_fragment == start.as_str() && end_fragment == end.as_str() {
                        start_token = start.to_string();
                        end_token = end.to_string();
                        line_commented = true;
                        break;
                    }
                }
            }

            if !line_commented {
                comment_changes.push(CommentChange::Uncommented {
                    range: *range,
                    start_pos,
                    end_pos,
                    start_token: default_tokens.start.clone(),
                    end_token: default_tokens.end.clone(),
                });
                commented = false;
            } else {
                comment_changes.push(CommentChange::Commented {
                    range: *range,
                    start_pos,
                    end_pos,
                    start_margin: selection_slice.get_char(after_start) == Some(' '),
                    end_margin: after_start != before_end
                        && (selection_slice.get_char(before_end) == Some(' ')),
                    start_token: start_token.to_string(),
                    end_token: end_token.to_string(),
                });
            }
            only_whitespace = false;
        } else {
            comment_changes.push(CommentChange::Whitespace { range: *range });
        }
    }
    if only_whitespace {
        commented = false;
    }
    (commented, comment_changes)
}

#[must_use]
pub fn create_block_comment_transaction(
    doc: &Rope,
    selection: &Selection,
    commented: bool,
    comment_changes: Vec<CommentChange>,
) -> (Transaction, SmallVec<[Range; 1]>) {
    let mut changes: Vec<Change> = Vec::with_capacity(selection.len() * 2);
    let mut ranges: SmallVec<[Range; 1]> = SmallVec::with_capacity(selection.len());
    let mut offs = 0;
    for change in comment_changes {
        if commented {
            if let CommentChange::Commented {
                range,
                start_pos,
                end_pos,
                start_token,
                end_token,
                start_margin,
                end_margin,
            } = change
            {
                let from = range.from();
                changes.push((
                    from + start_pos,
                    from + start_pos + start_token.len() + start_margin as usize,
                    None,
                ));
                changes.push((
                    from + end_pos - end_token.len() - end_margin as usize + 1,
                    from + end_pos + 1,
                    None,
                ));
            }
        } else {
            // uncommented so manually map ranges through changes
            match change {
                CommentChange::Uncommented {
                    range,
                    start_pos,
                    end_pos,
                    start_token,
                    end_token,
                } => {
                    let from = range.from();
                    changes.push((
                        from + start_pos,
                        from + start_pos,
                        Some(Tendril::from(format!("{} ", start_token))),
                    ));
                    changes.push((
                        from + end_pos + 1,
                        from + end_pos + 1,
                        Some(Tendril::from(format!(" {}", end_token))),
                    ));

                    let offset = start_token.chars().count() + end_token.chars().count() + 2;
                    ranges.push(
                        Range::new(from + offs, from + offs + end_pos + 1 + offset)
                            .with_direction(range.direction()),
                    );
                    offs += offset;
                }
                CommentChange::Commented { range, .. } | CommentChange::Whitespace { range } => {
                    ranges.push(Range::new(range.from() + offs, range.to() + offs));
                }
            }
        }
    }
    (Transaction::change(doc, changes.into_iter()), ranges)
}

#[must_use]
pub fn toggle_block_comments(
    doc: &Rope,
    selection: &Selection,
    tokens: &[BlockCommentToken],
) -> Transaction {
    let text = doc.slice(..);
    let (commented, comment_changes) = find_block_comments(tokens, text, selection);
    let (mut transaction, ranges) =
        create_block_comment_transaction(doc, selection, commented, comment_changes);
    if !commented {
        transaction = transaction.with_selection(Selection::new(ranges, selection.primary_index()));
    }
    transaction
}

pub fn split_lines_of_selection(text: RopeSlice, selection: &Selection) -> Selection {
    let mut ranges = SmallVec::new();
    for range in selection.ranges() {
        let (line_start, line_end) = range.line_range(text.slice(..));
        let mut pos = text.line_to_char(line_start);
        for line in text.slice(pos..text.line_to_char(line_end + 1)).lines() {
            let start = pos;
            pos += line.len_chars();
            ranges.push(Range::new(start, pos));
        }
    }
    Selection::new(ranges, 0)
}

#[cfg(test)]
mod test {
    use super::*;

    mod comment_box {
        use super::*;

        #[test]
        fn box_style_formats_to_eighty_columns() {
            let formatted = format_comment_box(
                "//",
                CommentBoxStyle::Box,
                CommentBoxAlignment::Left,
                DEFAULT_COMMENT_BOX_WIDTH,
                &["Parser".to_string()],
            );
            let lines = formatted.lines().collect::<Vec<_>>();

            assert_eq!(lines.len(), 3);
            assert!(lines.iter().all(|line| line.chars().count() == 80));
            assert_eq!(lines[0], format!("// +{}+", "-".repeat(75)));
            assert_eq!(lines[1], format!("// | ---- Parser {} |", "-".repeat(61)));
            assert_eq!(lines[2], format!("// +{}+", "-".repeat(75)));
        }

        #[test]
        fn centered_ruler_formats_to_eighty_columns() {
            let formatted = format_comment_box(
                "//",
                CommentBoxStyle::Ruler,
                CommentBoxAlignment::Center,
                DEFAULT_COMMENT_BOX_WIDTH,
                &["Parser".to_string()],
            );

            assert_eq!(formatted.chars().count(), 80);
            assert!(formatted.starts_with("// ---"));
            assert!(formatted.ends_with("---"));
            assert!(formatted.contains(" Parser "));
        }

        #[test]
        fn parses_builtin_styles() {
            assert_eq!("box".parse(), Ok(CommentBoxStyle::Box));
            assert_eq!("subheading".parse(), Ok(CommentBoxStyle::Subheading));
            assert_eq!("doc-heading".parse(), Ok(CommentBoxStyle::DocHeading));
            assert!("unknown".parse::<CommentBoxStyle>().is_err());
        }

        #[test]
        fn extracts_title_from_existing_box() {
            let formatted = format_comment_box(
                "//",
                CommentBoxStyle::Box,
                CommentBoxAlignment::Left,
                DEFAULT_COMMENT_BOX_WIDTH,
                &["Parser".to_string(), "Token recovery".to_string()],
            );

            assert_eq!(
                comment_box_text_lines("//", &formatted),
                vec!["Parser".to_string(), "Token recovery".to_string()]
            );
        }

        #[test]
        fn extracts_clean_title_from_all_builtin_styles() {
            for style in [
                CommentBoxStyle::Plain,
                CommentBoxStyle::Subheading,
                CommentBoxStyle::Heading,
                CommentBoxStyle::Ruler,
                CommentBoxStyle::Slash,
                CommentBoxStyle::Star,
                CommentBoxStyle::Hash,
                CommentBoxStyle::Box,
                CommentBoxStyle::Sidebar,
                CommentBoxStyle::Fold,
                CommentBoxStyle::DocHeading,
            ] {
                let formatted = format_comment_box(
                    "//",
                    style,
                    CommentBoxAlignment::Left,
                    DEFAULT_COMMENT_BOX_WIDTH,
                    &["Pool Methods".to_string()],
                );

                assert_eq!(
                    comment_box_text_lines("//", &formatted),
                    vec!["Pool Methods".to_string()],
                    "{style:?} should extract a clean title from {formatted:?}"
                );
            }
        }
    }

    mod find_line_comment {
        use super::*;

        #[test]
        fn not_commented() {
            // four lines, two space indented, except for line 1 which is blank.
            let doc = Rope::from("  1\n\n  2\n  3");

            let text = doc.slice(..);

            let res = find_line_comment("//", text, 0..3);
            // (commented = false, to_change = [line 0, line 2], min = col 2, margin = 0)
            assert_eq!(res, (false, vec![0, 2], 2, 0));
        }

        #[test]
        fn is_commented() {
            // three lines where the second line is empty.
            let doc = Rope::from("// hello\n\n// there");

            let res = find_line_comment("//", doc.slice(..), 0..3);

            // (commented = true, to_change = [line 0, line 2], min = col 0, margin = 1)
            assert_eq!(res, (true, vec![0, 2], 0, 1));
        }
    }

    // TODO: account for uncommenting with uneven comment indentation
    mod toggle_line_comment {
        use super::*;

        #[test]
        fn comment() {
            // four lines, two space indented, except for line 1 which is blank.
            let mut doc = Rope::from("  1\n\n  2\n  3");
            // select whole document
            let selection = Selection::single(0, doc.len_chars() - 1);

            let transaction = toggle_line_comments(&doc, &selection, None);
            transaction.apply(&mut doc);

            assert_eq!(doc, "  # 1\n\n  # 2\n  # 3");
        }

        #[test]
        fn uncomment() {
            let mut doc = Rope::from("  # 1\n\n  # 2\n  # 3");
            let mut selection = Selection::single(0, doc.len_chars() - 1);

            let transaction = toggle_line_comments(&doc, &selection, None);
            transaction.apply(&mut doc);
            selection = selection.map(transaction.changes());

            assert_eq!(doc, "  1\n\n  2\n  3");
            assert!(selection.len() == 1); // to ignore the selection unused warning
        }

        #[test]
        fn uncomment_0_margin_comments() {
            let mut doc = Rope::from("  #1\n\n  #2\n  #3");
            let mut selection = Selection::single(0, doc.len_chars() - 1);

            let transaction = toggle_line_comments(&doc, &selection, None);
            transaction.apply(&mut doc);
            selection = selection.map(transaction.changes());

            assert_eq!(doc, "  1\n\n  2\n  3");
            assert!(selection.len() == 1); // to ignore the selection unused warning
        }

        #[test]
        fn uncomment_0_margin_comments_with_no_space() {
            let mut doc = Rope::from("#");
            let mut selection = Selection::single(0, doc.len_chars() - 1);

            let transaction = toggle_line_comments(&doc, &selection, None);
            transaction.apply(&mut doc);
            selection = selection.map(transaction.changes());
            assert_eq!(doc, "");
            assert!(selection.len() == 1); // to ignore the selection unused warning
        }
    }

    #[test]
    fn test_find_block_comments() {
        // three lines 5 characters.
        let mut doc = Rope::from("1\n2\n3");
        // select whole document
        let selection = Selection::single(0, doc.len_chars());

        let text = doc.slice(..);

        let res = find_block_comments(&[BlockCommentToken::default()], text, &selection);

        assert_eq!(
            res,
            (
                false,
                vec![CommentChange::Uncommented {
                    range: Range::new(0, 5),
                    start_pos: 0,
                    end_pos: 4,
                    start_token: "/*".to_string(),
                    end_token: "*/".to_string(),
                }]
            )
        );

        // comment
        let transaction = toggle_block_comments(&doc, &selection, &[BlockCommentToken::default()]);
        transaction.apply(&mut doc);

        assert_eq!(doc, "/* 1\n2\n3 */");

        // uncomment
        let selection = Selection::single(0, doc.len_chars());
        let transaction = toggle_block_comments(&doc, &selection, &[BlockCommentToken::default()]);
        transaction.apply(&mut doc);
        assert_eq!(doc, "1\n2\n3");

        // don't panic when there is just a space in comment
        doc = Rope::from("/* */");
        let selection = Selection::single(0, doc.len_chars());
        let transaction = toggle_block_comments(&doc, &selection, &[BlockCommentToken::default()]);
        transaction.apply(&mut doc);
        assert_eq!(doc, "");
    }

    /// Test, if `get_comment_tokens` works, even if the content of the file includes chars, whose
    /// byte size unequal the amount of chars
    #[test]
    fn test_get_comment_with_char_boundaries() {
        let rope = Rope::from("··");
        let tokens = ["//", "///"];

        assert_eq!(
            super::get_comment_token(rope.slice(..), tokens.as_slice(), 0),
            None
        );
    }

    /// Test for `get_comment_token`.
    ///
    /// Assuming the comment tokens are stored as `["///", "//"]`, `get_comment_token` should still
    /// return `///` instead of `//` if the user is in a doc-comment section.
    #[test]
    fn test_use_longest_comment() {
        let text = Rope::from("    /// amogus");
        let tokens = ["///", "//"];

        assert_eq!(
            super::get_comment_token(text.slice(..), tokens.as_slice(), 0),
            Some("///")
        );
    }
}
