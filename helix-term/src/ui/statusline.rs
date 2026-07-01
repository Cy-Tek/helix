use helix_core::indent::IndentStyle;
use helix_core::{coords_at_pos, encoding, unicode::width::UnicodeWidthStr, Position};
use helix_lsp::lsp::DiagnosticSeverity;
use helix_view::document::DEFAULT_LANGUAGE_NAME;
use helix_view::editor::{StatusLineGlyphs, StatusLineStyle};
use helix_view::{
    document::{Mode, SCRATCH_BUFFER_NAME},
    graphics::{Color, Rect},
    theme::{Style, Theme},
    Document, Editor, View,
};

use crate::ui::ProgressSpinners;

use helix_view::editor::StatusLineElement as StatusLineElementID;
use tui::buffer::Buffer as Surface;
use tui::text::{Span, Spans};

pub struct RenderContext<'a> {
    pub editor: &'a Editor,
    pub doc: &'a Document,
    pub view: &'a View,
    pub focused: bool,
    pub spinners: &'a ProgressSpinners,
    pub parts: RenderBuffer<'a>,
}

impl<'a> RenderContext<'a> {
    pub fn new(
        editor: &'a Editor,
        doc: &'a Document,
        view: &'a View,
        focused: bool,
        spinners: &'a ProgressSpinners,
    ) -> Self {
        RenderContext {
            editor,
            doc,
            view,
            focused,
            spinners,
            parts: RenderBuffer::default(),
        }
    }
}

#[derive(Default)]
pub struct RenderBuffer<'a> {
    pub left: Spans<'a>,
    pub center: Spans<'a>,
    pub right: Spans<'a>,
}

pub fn render(context: &mut RenderContext, viewport: Rect, surface: &mut Surface) {
    if context.editor.config().statusline.style == StatusLineStyle::Capsule {
        render_capsule(context, viewport, surface);
        return;
    }

    let base_style = if context.focused {
        context.editor.theme.get("ui.statusline")
    } else {
        context.editor.theme.get("ui.statusline.inactive")
    };

    surface.set_style(viewport.with_height(1), base_style);

    // Left side of the status line.

    let config = context.editor.config();

    for element_id in &config.statusline.left {
        let render = get_render_function(*element_id);
        (render)(context, |context, span| {
            append(&mut context.parts.left, span, base_style)
        });
    }

    surface.set_spans(
        viewport.x,
        viewport.y,
        &context.parts.left,
        context.parts.left.width() as u16,
    );

    // Right side of the status line.

    for element_id in &config.statusline.right {
        let render = get_render_function(*element_id);
        (render)(context, |context, span| {
            append(&mut context.parts.right, span, base_style)
        })
    }

    surface.set_spans(
        viewport.x
            + viewport
                .width
                .saturating_sub(context.parts.right.width() as u16),
        viewport.y,
        &context.parts.right,
        context.parts.right.width() as u16,
    );

    // Center of the status line.

    for element_id in &config.statusline.center {
        let render = get_render_function(*element_id);
        (render)(context, |context, span| {
            append(&mut context.parts.center, span, base_style)
        })
    }

    // Width of the empty space between the left and center area and between the center and right area.
    let spacing = 1u16;

    let edge_width = context.parts.left.width().max(context.parts.right.width()) as u16;
    let center_max_width = viewport.width.saturating_sub(2 * edge_width + 2 * spacing);
    let center_width = center_max_width.min(context.parts.center.width() as u16);

    surface.set_spans(
        viewport.x + viewport.width / 2 - center_width / 2,
        viewport.y,
        &context.parts.center,
        center_width,
    );
}

fn append<'a>(buffer: &mut Spans<'a>, mut span: Span<'a>, base_style: Style) {
    span.style = base_style.patch(span.style);
    buffer.0.push(span);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CapsuleGlyphs {
    pub(super) left_cap: &'static str,
    pub(super) right_cap: &'static str,
    pub(super) separator: &'static str,
    pub(super) git_icon: &'static str,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
struct CapsuleFooterSegments {
    left: Vec<String>,
    right: Vec<String>,
}

pub(super) fn capsule_glyphs(glyphs: StatusLineGlyphs) -> CapsuleGlyphs {
    match glyphs {
        StatusLineGlyphs::Nerd => CapsuleGlyphs {
            left_cap: "",
            right_cap: "",
            separator: "✦",
            git_icon: "",
        },
        StatusLineGlyphs::Plain => CapsuleGlyphs {
            left_cap: "(",
            right_cap: ")",
            separator: "*",
            git_icon: "git",
        },
    }
}

pub(super) fn capsule_text(glyphs: CapsuleGlyphs, label: &str) -> String {
    format!("{} {} {}", glyphs.left_cap, label, glyphs.right_cap)
}

const CAPSULE_EDGE_PADDING: u16 = 1;

pub(super) fn capsule_left_x(viewport: Rect) -> u16 {
    if viewport.width > CAPSULE_EDGE_PADDING {
        viewport.x.saturating_add(CAPSULE_EDGE_PADDING)
    } else {
        viewport.x
    }
}

pub(super) fn capsule_right_x(viewport: Rect, width: u16) -> u16 {
    viewport.x
        + viewport
            .width
            .saturating_sub(width.saturating_add(CAPSULE_EDGE_PADDING))
}

#[cfg(test)]
fn capsule_footer_segments(
    glyphs: StatusLineGlyphs,
    mode: &str,
    language: &str,
    diagnostics: Option<&str>,
    branch: Option<&str>,
    position: &str,
) -> CapsuleFooterSegments {
    let glyphs = capsule_glyphs(glyphs);
    let mut left = vec![capsule_text(glyphs, mode), glyphs.separator.into()];
    left.push(capsule_text(glyphs, language));
    if let Some(diagnostics) = diagnostics {
        left.push(glyphs.separator.into());
        left.push(capsule_text(glyphs, diagnostics));
    }

    let mut right = Vec::new();
    if let Some(branch) = branch {
        right.push(capsule_text(
            glyphs,
            &format!("{} {}", glyphs.git_icon, branch),
        ));
        right.push(glyphs.separator.into());
    }
    right.push(capsule_text(glyphs, position));

    CapsuleFooterSegments { left, right }
}

fn render_capsule(context: &mut RenderContext, viewport: Rect, surface: &mut Surface) {
    let base_style = if context.focused {
        context
            .editor
            .theme
            .try_get_exact("ui.statusline.capsule")
            .unwrap_or_else(|| context.editor.theme.get("ui.statusline"))
    } else {
        context.editor.theme.get("ui.statusline.inactive")
    };
    surface.set_style(viewport.with_height(1), base_style);

    let glyphs = capsule_glyphs(context.editor.config().statusline.glyphs);
    let mode = capsule_mode_label(context);
    let language = context.doc.language_name().unwrap_or(DEFAULT_LANGUAGE_NAME);
    let diagnostics = capsule_diagnostics_label(context);
    let branch = context
        .doc
        .version_control_head()
        .map(|head| head.to_string())
        .filter(|head| !head.is_empty());
    let position = capsule_position_label(context);

    let accent_style = capsule_accent_style(context, base_style);
    let mut left = Spans::default();
    push_capsule(
        &mut left,
        glyphs,
        &mode,
        capsule_mode_style(context, base_style),
        base_style,
    );
    push_capsule_separator(&mut left, glyphs, accent_style, base_style);
    push_capsule(
        &mut left,
        glyphs,
        language,
        capsule_file_style(context, base_style),
        base_style,
    );
    if let Some(diagnostics) = diagnostics.as_deref() {
        push_capsule_separator(&mut left, glyphs, accent_style, base_style);
        push_capsule(
            &mut left,
            glyphs,
            diagnostics,
            capsule_meta_style(context, base_style),
            base_style,
        );
    }

    let mut right = Spans::default();
    if let Some(branch) = branch.as_deref() {
        push_capsule(
            &mut right,
            glyphs,
            &format!("{} {}", glyphs.git_icon, branch),
            capsule_project_style(context, base_style),
            base_style,
        );
        push_capsule_separator(&mut right, glyphs, accent_style, base_style);
    }
    push_capsule(
        &mut right,
        glyphs,
        &position,
        capsule_meta_style(context, base_style),
        base_style,
    );

    surface.set_spans(
        capsule_left_x(viewport),
        viewport.y,
        &left,
        left.width() as u16,
    );
    surface.set_spans(
        capsule_right_x(viewport, right.width() as u16),
        viewport.y,
        &right,
        right.width() as u16,
    );
}

fn capsule_mode_label(context: &RenderContext) -> String {
    let mode = &context.editor.config().statusline.mode;
    match context.editor.mode() {
        Mode::Insert => mode.insert.clone(),
        Mode::Select => mode.select.clone(),
        Mode::Normal => mode.normal.clone(),
    }
}

fn capsule_position_label(context: &RenderContext) -> String {
    let position = get_position(context);
    let maxrows = context.doc.text().len_lines();
    let percent = (position.row + 1) * 100 / maxrows;
    format!("{}:{} · {}%", position.row + 1, position.col + 1, percent)
}

fn capsule_diagnostics_label(context: &RenderContext) -> Option<String> {
    use helix_core::diagnostic::Severity;
    let (hints, info, warnings, errors) =
        context
            .doc
            .diagnostics()
            .iter()
            .fold((0, 0, 0, 0), |mut counts, diag| {
                match diag.severity {
                    Some(Severity::Hint) | None => counts.0 += 1,
                    Some(Severity::Info) => counts.1 += 1,
                    Some(Severity::Warning) => counts.2 += 1,
                    Some(Severity::Error) => counts.3 += 1,
                }
                counts
            });

    let mut parts = Vec::new();
    for severity in &context.editor.config().statusline.diagnostics {
        match severity {
            Severity::Hint if hints > 0 => parts.push(format!("hints {hints}")),
            Severity::Info if info > 0 => parts.push(format!("info {info}")),
            Severity::Warning if warnings > 0 => parts.push(format!("warnings {warnings}")),
            Severity::Error if errors > 0 => parts.push(format!("errors {errors}")),
            _ => {}
        }
    }

    (!parts.is_empty()).then(|| format!("⚠ {}", parts.join(" ")))
}

fn first_theme_style(editor: &Editor, keys: &[&str], fallback: Style) -> Style {
    keys.iter()
        .find_map(|key| editor.theme.try_get(key))
        .unwrap_or(fallback)
}

fn capsule_style(
    context: &RenderContext,
    key: &str,
    line_style: Style,
    fallback: Style,
    accent: Style,
) -> Style {
    capsule_style_from_theme(&context.editor.theme, key, line_style, fallback, accent)
}

fn capsule_style_from_theme(
    theme: &Theme,
    key: &str,
    line_style: Style,
    fallback: Style,
    accent: Style,
) -> Style {
    capsule_theme_style(theme, key)
        .unwrap_or_else(|| capsule_contrast_style(line_style, fallback, accent))
}

pub(super) fn capsule_theme_style(theme: &Theme, key: &str) -> Option<Style> {
    theme.try_get_exact(key)
}

fn capsule_mode_style(context: &RenderContext, line_style: Style) -> Style {
    let fallback = if context.editor.config().color_modes {
        match context.editor.mode() {
            Mode::Insert => context.editor.theme.get("ui.statusline.insert"),
            Mode::Select => context.editor.theme.get("ui.statusline.select"),
            Mode::Normal => context.editor.theme.get("ui.statusline.normal"),
        }
    } else {
        context.editor.theme.get("ui.statusline.normal")
    };
    capsule_style(
        context,
        "ui.statusline.capsule.mode",
        line_style,
        fallback,
        fallback,
    )
}

fn capsule_file_style(context: &RenderContext, line_style: Style) -> Style {
    let fallback = first_theme_style(
        context.editor,
        &["ui.statusline.insert", "ui.cursor", "ui.text.focus"],
        context.editor.theme.get("ui.statusline"),
    );
    capsule_style(
        context,
        "ui.statusline.capsule.file",
        line_style,
        fallback,
        fallback,
    )
}

fn capsule_project_style(context: &RenderContext, line_style: Style) -> Style {
    let fallback = first_theme_style(
        context.editor,
        &["ui.statusline.normal", "ui.text.focus"],
        context.editor.theme.get("ui.statusline"),
    );
    capsule_style(
        context,
        "ui.statusline.capsule.project",
        line_style,
        fallback,
        fallback,
    )
}

fn capsule_meta_style(context: &RenderContext, line_style: Style) -> Style {
    let fallback = first_theme_style(
        context.editor,
        &[
            "ui.statusline.select",
            "ui.selection.primary",
            "ui.statusline.inactive",
        ],
        context.editor.theme.get("ui.statusline"),
    );
    capsule_style(
        context,
        "ui.statusline.capsule.meta",
        line_style,
        fallback,
        fallback,
    )
}

fn capsule_accent_style(context: &RenderContext, line_style: Style) -> Style {
    capsule_theme_style(&context.editor.theme, "ui.statusline.capsule.accent").unwrap_or_else(
        || {
            first_theme_style(
                context.editor,
                &[
                    "ui.text.focus",
                    "ui.statusline.normal",
                    "ui.statusline.separator",
                ],
                line_style,
            )
        },
    )
}

pub(super) fn capsule_contrast_style(line_style: Style, fallback: Style, accent: Style) -> Style {
    let mut style = fallback;
    let needs_background = style.bg.is_none() || style.bg == line_style.bg;

    if needs_background {
        if let Some(background) = accent.bg.or(accent.fg).or(fallback.fg).or(line_style.fg) {
            style.bg = Some(background);
            style.fg = line_style.bg.or(line_style.fg);
        }
    }

    if style.fg.is_none() {
        style.fg = line_style.fg;
    }

    if style.fg == style.bg {
        style.fg = line_style.bg.or(line_style.fg);
    }

    style
}

pub(super) fn push_capsule<'a>(
    spans: &mut Spans<'a>,
    glyphs: CapsuleGlyphs,
    label: &str,
    body_style: Style,
    line_style: Style,
) {
    if glyphs.left_cap == "(" {
        spans
            .0
            .push(Span::styled(capsule_text(glyphs, label), body_style));
        return;
    }

    let cap_color = body_style.bg.or(body_style.fg).unwrap_or(Color::Reset);
    let cap_style = Style::default()
        .fg(cap_color)
        .bg(line_style.bg.unwrap_or(Color::Reset));
    spans
        .0
        .push(Span::styled(glyphs.left_cap.to_string(), cap_style));
    spans.0.push(Span::styled(format!(" {label} "), body_style));
    spans
        .0
        .push(Span::styled(glyphs.right_cap.to_string(), cap_style));
}

pub(super) fn push_capsule_separator<'a>(
    spans: &mut Spans<'a>,
    glyphs: CapsuleGlyphs,
    accent_style: Style,
    line_style: Style,
) {
    // The Nerd separator (✦) is drawn ~2 cells wide by the font, but the layout
    // only reserves 1 cell — so its right half bleeds into the trailing space
    // and the glyph ends up flush against the next capsule. Pad the right with
    // an extra space so it reads as visually centered between the pills. The
    // Plain separator (*) is a normal 1-cell glyph and stays symmetric.
    let separator = if glyphs.left_cap == "(" {
        format!(" {} ", glyphs.separator)
    } else {
        format!(" {}  ", glyphs.separator)
    };
    spans
        .0
        .push(Span::styled(separator, line_style.patch(accent_style)));
}

fn get_render_function<'a, F>(element_id: StatusLineElementID) -> impl Fn(&mut RenderContext<'a>, F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    match element_id {
        helix_view::editor::StatusLineElement::Mode => render_mode,
        helix_view::editor::StatusLineElement::Spinner => render_lsp_spinner,
        helix_view::editor::StatusLineElement::FileBaseName => render_file_base_name,
        helix_view::editor::StatusLineElement::FileName => render_file_name,
        helix_view::editor::StatusLineElement::FileAbsolutePath => render_file_absolute_path,
        helix_view::editor::StatusLineElement::FileModificationIndicator => {
            render_file_modification_indicator
        }
        helix_view::editor::StatusLineElement::ReadOnlyIndicator => render_read_only_indicator,
        helix_view::editor::StatusLineElement::FileEncoding => render_file_encoding,
        helix_view::editor::StatusLineElement::FileLineEnding => render_file_line_ending,
        helix_view::editor::StatusLineElement::FileIndentStyle => render_file_indent_style,
        helix_view::editor::StatusLineElement::FileType => render_file_type,
        helix_view::editor::StatusLineElement::Diagnostics => render_diagnostics,
        helix_view::editor::StatusLineElement::WorkspaceDiagnostics => render_workspace_diagnostics,
        helix_view::editor::StatusLineElement::Selections => render_selections,
        helix_view::editor::StatusLineElement::PrimarySelectionLength => {
            render_primary_selection_length
        }
        helix_view::editor::StatusLineElement::Position => render_position,
        helix_view::editor::StatusLineElement::PositionPercentage => render_position_percentage,
        helix_view::editor::StatusLineElement::TotalLineNumbers => render_total_line_numbers,
        helix_view::editor::StatusLineElement::Separator => render_separator,
        helix_view::editor::StatusLineElement::Spacer => render_spacer,
        helix_view::editor::StatusLineElement::VersionControl => render_version_control,
        helix_view::editor::StatusLineElement::Register => render_register,
        helix_view::editor::StatusLineElement::CurrentWorkingDirectory => render_cwd,
    }
}

fn render_mode<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let visible = context.focused;
    let config = context.editor.config();
    let modenames = &config.statusline.mode;
    let mode_str = match context.editor.mode() {
        Mode::Insert => &modenames.insert,
        Mode::Select => &modenames.select,
        Mode::Normal => &modenames.normal,
    };
    let content = if visible {
        format!(" {mode_str} ")
    } else {
        // If not focused, explicitly leave an empty space instead of returning None.
        " ".repeat(mode_str.width() + 2)
    };
    let style = if visible && config.color_modes {
        match context.editor.mode() {
            Mode::Insert => context.editor.theme.get("ui.statusline.insert"),
            Mode::Select => context.editor.theme.get("ui.statusline.select"),
            Mode::Normal => context.editor.theme.get("ui.statusline.normal"),
        }
    } else {
        Style::default()
    };
    write(context, Span::styled(content, style));
}

fn render_lsp_spinner<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    write(
        context,
        context
            .doc
            .language_servers()
            .find_map(|srv| {
                context
                    .spinners
                    .get(srv.id())
                    .and_then(|spinner| spinner.frame())
            })
            // Even if there's no spinner; reserve its space to avoid elements frequently shifting.
            .unwrap_or(" ")
            .into(),
    );
}

fn render_diagnostics<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    use helix_core::diagnostic::Severity;
    let (hints, info, warnings, errors) =
        context
            .doc
            .diagnostics()
            .iter()
            .fold((0, 0, 0, 0), |mut counts, diag| {
                match diag.severity {
                    Some(Severity::Hint) | None => counts.0 += 1,
                    Some(Severity::Info) => counts.1 += 1,
                    Some(Severity::Warning) => counts.2 += 1,
                    Some(Severity::Error) => counts.3 += 1,
                }
                counts
            });

    for sev in &context.editor.config().statusline.diagnostics {
        match sev {
            Severity::Hint if hints > 0 => {
                write(context, Span::styled("●", context.editor.theme.get("hint")));
                write(context, format!(" {} ", hints).into());
            }
            Severity::Info if info > 0 => {
                write(context, Span::styled("●", context.editor.theme.get("info")));
                write(context, format!(" {} ", info).into());
            }
            Severity::Warning if warnings > 0 => {
                write(
                    context,
                    Span::styled("●", context.editor.theme.get("warning")),
                );
                write(context, format!(" {} ", warnings).into());
            }
            Severity::Error if errors > 0 => {
                write(
                    context,
                    Span::styled("●", context.editor.theme.get("error")),
                );
                write(context, format!(" {} ", errors).into());
            }
            _ => {}
        }
    }
}

fn render_workspace_diagnostics<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    use helix_core::diagnostic::Severity;
    let (hints, info, warnings, errors) = context.editor.diagnostics.values().flatten().fold(
        (0u32, 0u32, 0u32, 0u32),
        |mut counts, (diag, _)| {
            match diag.severity {
                // PERF: For large workspace diagnostics, this loop can be very tight.
                //
                // Most often the diagnostics will be for warnings and errors.
                // Errors should tend to be fixed fast, leaving warnings as the most common.
                Some(DiagnosticSeverity::WARNING) => counts.2 += 1,
                Some(DiagnosticSeverity::ERROR) => counts.3 += 1,
                Some(DiagnosticSeverity::HINT) => counts.0 += 1,
                Some(DiagnosticSeverity::INFORMATION) => counts.1 += 1,
                // Fallback to `hint`.
                _ => counts.0 += 1,
            }
            counts
        },
    );

    let sevs_to_show = &context.editor.config().statusline.workspace_diagnostics;

    // Avoid showing the " W " if no diagnostic counts will be shown.
    if !sevs_to_show.iter().any(|sev| match sev {
        Severity::Hint => hints != 0,
        Severity::Info => info != 0,
        Severity::Warning => warnings != 0,
        Severity::Error => errors != 0,
    }) {
        return;
    }

    write(context, " W ".into());

    for sev in sevs_to_show {
        match sev {
            Severity::Hint if hints > 0 => {
                write(context, Span::styled("●", context.editor.theme.get("hint")));
                write(context, format!(" {} ", hints).into());
            }
            Severity::Info if info > 0 => {
                write(context, Span::styled("●", context.editor.theme.get("info")));
                write(context, format!(" {} ", info).into());
            }
            Severity::Warning if warnings > 0 => {
                write(
                    context,
                    Span::styled("●", context.editor.theme.get("warning")),
                );
                write(context, format!(" {} ", warnings).into());
            }
            Severity::Error if errors > 0 => {
                write(
                    context,
                    Span::styled("●", context.editor.theme.get("error")),
                );
                write(context, format!(" {} ", errors).into());
            }
            _ => {}
        }
    }
}

fn render_selections<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let selection = context.doc.selection(context.view.id);
    let count = selection.len();
    write(
        context,
        if count == 1 {
            " 1 sel ".into()
        } else {
            format!(" {}/{count} sels ", selection.primary_index() + 1).into()
        },
    );
}

fn render_primary_selection_length<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let tot_sel = context.doc.selection(context.view.id).primary().len();
    write(
        context,
        format!(" {} char{} ", tot_sel, if tot_sel == 1 { "" } else { "s" }).into(),
    );
}

fn get_position(context: &RenderContext) -> Position {
    coords_at_pos(
        context.doc.text().slice(..),
        context
            .doc
            .selection(context.view.id)
            .primary()
            .cursor(context.doc.text().slice(..)),
    )
}

fn render_position<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let position = get_position(context);
    write(
        context,
        format!(" {}:{} ", position.row + 1, position.col + 1).into(),
    );
}

fn render_total_line_numbers<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let total_line_numbers = context.doc.text().len_lines();

    write(context, format!(" {} ", total_line_numbers).into());
}

fn render_position_percentage<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let position = get_position(context);
    let maxrows = context.doc.text().len_lines();
    write(
        context,
        format!("{}%", (position.row + 1) * 100 / maxrows).into(),
    );
}

fn render_file_encoding<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let enc = context.doc.encoding();

    if enc != encoding::UTF_8 {
        write(context, format!(" {} ", enc.name()).into());
    }
}

fn render_file_line_ending<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    use helix_core::LineEnding::*;
    let line_ending = match context.doc.line_ending {
        Crlf => "CRLF",
        LF => "LF",
        #[cfg(feature = "unicode-lines")]
        VT => "VT", // U+000B -- VerticalTab
        #[cfg(feature = "unicode-lines")]
        FF => "FF", // U+000C -- FormFeed
        #[cfg(feature = "unicode-lines")]
        CR => "CR", // U+000D -- CarriageReturn
        #[cfg(feature = "unicode-lines")]
        Nel => "NEL", // U+0085 -- NextLine
        #[cfg(feature = "unicode-lines")]
        LS => "LS", // U+2028 -- Line Separator
        #[cfg(feature = "unicode-lines")]
        PS => "PS", // U+2029 -- ParagraphSeparator
    };

    write(context, format!(" {} ", line_ending).into());
}

fn render_file_type<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let file_type = context.doc.language_name().unwrap_or(DEFAULT_LANGUAGE_NAME);

    write(context, format!(" {} ", file_type).into());
}

fn render_file_name<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let title = if let Some(name) = context.editor.doc_display_name(context.doc.id()) {
        format!(" {} ", name)
    } else {
        let rel_path = context.doc.relative_path();
        let path = rel_path
            .as_ref()
            .map(|p| p.to_string_lossy())
            .unwrap_or_else(|| SCRATCH_BUFFER_NAME.into());
        format!(" {} ", path)
    };

    write(context, title.into());
}

fn render_file_absolute_path<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let title = {
        let path = context
            .doc
            .path()
            .as_ref()
            .map_or_else(|| SCRATCH_BUFFER_NAME.into(), |p| p.to_string_lossy());
        format!(" {} ", path)
    };

    write(context, title.into());
}

fn render_file_modification_indicator<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let title = if context.doc.is_modified() {
        "[+]"
    } else {
        "   "
    };

    write(context, title.into());
}

fn render_read_only_indicator<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let title = if context.doc.readonly {
        " [readonly] "
    } else {
        ""
    };
    write(context, title.into());
}

fn render_file_base_name<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let title = if let Some(name) = context.editor.doc_display_name(context.doc.id()) {
        format!(" {} ", name)
    } else {
        let rel_path = context.doc.relative_path();
        let path = rel_path
            .as_ref()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy()))
            .unwrap_or_else(|| SCRATCH_BUFFER_NAME.into());
        format!(" {} ", path)
    };

    write(context, title.into());
}

fn render_separator<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let sep = &context.editor.config().statusline.separator;
    let style = context.editor.theme.get("ui.statusline.separator");

    write(context, Span::styled(sep.to_string(), style));
}

fn render_spacer<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    write(context, " ".into());
}

fn render_version_control<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let head = context
        .doc
        .version_control_head()
        .unwrap_or_default()
        .to_string();

    write(context, head.into());
}

fn render_register<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    if let Some(reg) = context.editor.selected_register {
        write(context, format!(" reg={} ", reg).into())
    }
}

fn render_file_indent_style<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let style = context.doc.indent_style;

    write(
        context,
        match style {
            IndentStyle::Tabs => " tabs ".into(),
            IndentStyle::Spaces(indent) => {
                format!(" {} space{} ", indent, if indent == 1 { "" } else { "s" }).into()
            }
        },
    );
}

fn render_cwd<'a, F>(context: &mut RenderContext<'a>, write: F)
where
    F: Fn(&mut RenderContext<'a>, Span<'a>) + Copy,
{
    let cwd = helix_stdx::env::current_working_dir();
    let cwd = cwd
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    write(context, cwd.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_view::editor::StatusLineGlyphs;

    #[test]
    fn capsule_nerd_glyphs_use_powerline_caps_and_stars() {
        let glyphs = capsule_glyphs(StatusLineGlyphs::Nerd);

        assert_eq!(glyphs.left_cap, "");
        assert_eq!(glyphs.right_cap, "");
        assert_eq!(glyphs.separator, "✦");
        assert_eq!(capsule_text(glyphs, "NORMAL"), " NORMAL ");
    }

    #[test]
    fn capsule_plain_glyphs_avoid_nerd_font_symbols() {
        let glyphs = capsule_glyphs(StatusLineGlyphs::Plain);
        let text = capsule_text(glyphs, "NORMAL");

        assert_eq!(text, "( NORMAL )");
        assert!(!text.contains(''));
        assert_eq!(glyphs.separator, "*");
    }

    #[test]
    fn capsule_footer_segments_include_mode_language_branch_and_position() {
        let segments = capsule_footer_segments(
            StatusLineGlyphs::Nerd,
            "NORMAL",
            "C3",
            Some("⚠ warnings 2"),
            Some("main"),
            "44:1 · 62%",
        );

        assert_eq!(
            segments.left,
            vec![" NORMAL ", "✦", " C3 ", "✦", " ⚠ warnings 2 "]
        );
        assert_eq!(segments.right, vec!["  main ", "✦", " 44:1 · 62% "]);
    }

    #[test]
    fn capsule_left_x_leaves_one_cell_at_terminal_edge() {
        let viewport = Rect::new(0, 0, 100, 1);

        assert_eq!(capsule_left_x(viewport), 1);
    }

    #[test]
    fn capsule_right_x_leaves_one_cell_at_terminal_edge() {
        let viewport = Rect::new(0, 0, 100, 1);

        assert_eq!(capsule_right_x(viewport, 12), 87);
    }

    #[test]
    fn capsule_edge_padding_does_not_underflow_tiny_viewports() {
        let viewport = Rect::new(7, 0, 0, 1);

        assert_eq!(capsule_left_x(viewport), 7);
        assert_eq!(capsule_right_x(viewport, 12), 7);
    }

    #[test]
    fn capsule_contrast_style_uses_accent_when_fallback_matches_line_background() {
        let line = Style::default().fg(Color::White).bg(Color::Black);
        let fallback = Style::default().fg(Color::White).bg(Color::Black);
        let accent = Style::default().fg(Color::Yellow);

        let style = capsule_contrast_style(line, fallback, accent);

        assert_eq!(style.bg, Some(Color::Yellow));
        assert_eq!(style.fg, Some(Color::Black));
    }

    #[test]
    fn capsule_contrast_style_preserves_existing_distinct_background() {
        let line = Style::default().fg(Color::White).bg(Color::Black);
        let fallback = Style::default().fg(Color::White).bg(Color::Blue);
        let accent = Style::default().fg(Color::Yellow);

        let style = capsule_contrast_style(line, fallback, accent);

        assert_eq!(style.bg, Some(Color::Blue));
        assert_eq!(style.fg, Some(Color::White));
    }

    #[test]
    fn capsule_theme_style_requires_exact_capsule_keys() {
        let theme = toml::from_str::<Theme>(
            r##"
            "ui.statusline" = { fg = "#ffffff", bg = "#000000" }
            "ui.statusline.normal" = { fg = "#000000", bg = "#0000ff" }
            "##,
        )
        .unwrap();

        assert!(theme.try_get("ui.statusline.capsule.mode").is_some());
        assert!(capsule_theme_style(&theme, "ui.statusline.capsule.mode").is_none());

        let style = capsule_style_from_theme(
            &theme,
            "ui.statusline.capsule.mode",
            theme.get("ui.statusline"),
            theme.get("ui.statusline.normal"),
            theme.get("ui.statusline.normal"),
        );

        assert_eq!(style, theme.get("ui.statusline.normal"));
    }
}
