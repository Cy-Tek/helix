//! Helpers for the kitty graphics protocol's [Unicode placeholder] mechanism.
//!
//! Instead of placing an image at an absolute cursor position (which a multiplexer like tmux can't
//! track, so the image doesn't scroll/clip/redraw with the pane), the image is transmitted once
//! and a *virtual placement* is created. The image is then displayed by writing placeholder cells
//! (the character `U+10EEEE`) into the normal text grid: each cell encodes its row and column via
//! combining diacritics, and the image id via the cell's foreground color. Because the placeholder
//! cells are ordinary text, tmux tracks them like any glyph — solving scrolling, clipping and
//! redraws — and the host terminal (kitty, Ghostty, …) reconstructs the image over them.
//!
//! A nice side effect: an image can be drawn one row at a time, so a diagram taller than the
//! viewport (or only partially scrolled into view) still renders correctly — we simply emit the
//! placeholder rows that are visible.
//!
//! [Unicode placeholder]: https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders

use helix_view::graphics::{Color, Style};
use tui::buffer::Buffer as Surface;

/// The placeholder character used to mark image cells.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// The foreground color encoding the low 24 bits of an image id, as required by the Unicode
/// placeholder mechanism. (Ids using the high byte additionally need a third diacritic; see
/// [`placeholder_row`].)
pub fn image_id_color(id: u32) -> Color {
    Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8)
}

/// Build a single row of placeholder graphemes for image `id`: `cols` copies of `U+10EEEE`, each
/// carrying the row diacritic for `image_row` and the column diacritic for its position (and, when
/// the id uses its high byte, a third diacritic encoding it). The returned string is meant to be
/// written with [`image_id_color(id)`](image_id_color) as the foreground.
pub fn placeholder_row(image_row: u16, cols: u16, id: u32) -> String {
    let row_diacritic = ROWCOLUMN_DIACRITICS[(image_row as usize).min(ROWCOLUMN_DIACRITICS.len() - 1)];
    let high_byte = (id >> 24) as usize;
    let high_diacritic = (high_byte != 0).then(|| ROWCOLUMN_DIACRITICS[high_byte.min(ROWCOLUMN_DIACRITICS.len() - 1)]);

    let mut row = String::with_capacity(cols as usize * 12);
    for col in 0..cols {
        row.push(PLACEHOLDER);
        row.push(row_diacritic);
        row.push(ROWCOLUMN_DIACRITICS[(col as usize).min(ROWCOLUMN_DIACRITICS.len() - 1)]);
        if let Some(high) = high_diacritic {
            row.push(high);
        }
    }
    row
}

/// Write `n_rows` rows of placeholder cells for image `id` into `surface`, starting at screen
/// position `(x, y)` and at image row `image_row_start`. `cols` is the image's width in cells.
///
/// This is the display half of the protocol; the image data itself must be transmitted separately
/// (with a virtual placement of the matching `cols`×total-rows size) via the media channel.
#[allow(clippy::too_many_arguments)]
pub fn place_image_rows(
    surface: &mut Surface,
    x: u16,
    y: u16,
    cols: u16,
    image_row_start: u16,
    n_rows: u16,
    id: u32,
) {
    let style = Style::default().fg(image_id_color(id));
    for row in 0..n_rows {
        let line = placeholder_row(image_row_start + row, cols, id);
        surface.set_string(x, y + row, &line, style);
    }
}

#[rustfmt::skip]
// 297 row/column placeholder diacritics, derived from kitty gen/rowcolumn-diacritics.txt.
// Index N encodes row/column value N for the Unicode placeholder graphics mechanism.
pub const ROWCOLUMN_DIACRITICS: [char; 297] = [
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}',
    '\u{033F}', '\u{0346}', '\u{034A}', '\u{034B}', '\u{034C}', '\u{0350}', '\u{0351}',
    '\u{0352}', '\u{0357}', '\u{035B}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}',
    '\u{0367}', '\u{0368}', '\u{0369}', '\u{036A}', '\u{036B}', '\u{036C}', '\u{036D}',
    '\u{036E}', '\u{036F}', '\u{0483}', '\u{0484}', '\u{0485}', '\u{0486}', '\u{0487}',
    '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}', '\u{0598}', '\u{0599}',
    '\u{059C}', '\u{059D}', '\u{059E}', '\u{059F}', '\u{05A0}', '\u{05A1}', '\u{05A8}',
    '\u{05A9}', '\u{05AB}', '\u{05AC}', '\u{05AF}', '\u{05C4}', '\u{0610}', '\u{0611}',
    '\u{0612}', '\u{0613}', '\u{0614}', '\u{0615}', '\u{0616}', '\u{0617}', '\u{0657}',
    '\u{0658}', '\u{0659}', '\u{065A}', '\u{065B}', '\u{065D}', '\u{065E}', '\u{06D6}',
    '\u{06D7}', '\u{06D8}', '\u{06D9}', '\u{06DA}', '\u{06DB}', '\u{06DC}', '\u{06DF}',
    '\u{06E0}', '\u{06E1}', '\u{06E2}', '\u{06E4}', '\u{06E7}', '\u{06E8}', '\u{06EB}',
    '\u{06EC}', '\u{0730}', '\u{0732}', '\u{0733}', '\u{0735}', '\u{0736}', '\u{073A}',
    '\u{073D}', '\u{073F}', '\u{0740}', '\u{0741}', '\u{0743}', '\u{0745}', '\u{0747}',
    '\u{0749}', '\u{074A}', '\u{07EB}', '\u{07EC}', '\u{07ED}', '\u{07EE}', '\u{07EF}',
    '\u{07F0}', '\u{07F1}', '\u{07F3}', '\u{0816}', '\u{0817}', '\u{0818}', '\u{0819}',
    '\u{081B}', '\u{081C}', '\u{081D}', '\u{081E}', '\u{081F}', '\u{0820}', '\u{0821}',
    '\u{0822}', '\u{0823}', '\u{0825}', '\u{0826}', '\u{0827}', '\u{0829}', '\u{082A}',
    '\u{082B}', '\u{082C}', '\u{082D}', '\u{0951}', '\u{0953}', '\u{0954}', '\u{0F82}',
    '\u{0F83}', '\u{0F86}', '\u{0F87}', '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}',
    '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}', '\u{1A77}', '\u{1A78}', '\u{1A79}',
    '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}', '\u{1B6E}', '\u{1B6F}',
    '\u{1B70}', '\u{1B71}', '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}', '\u{1CD2}',
    '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}',
    '\u{1DD1}', '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}', '\u{1DD6}', '\u{1DD7}',
    '\u{1DD8}', '\u{1DD9}', '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}',
    '\u{1DDF}', '\u{1DE0}', '\u{1DE1}', '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}',
    '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}', '\u{20D4}', '\u{20D5}', '\u{20D6}',
    '\u{20D7}', '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}', '\u{20E9}', '\u{20F0}',
    '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}', '\u{2DE3}',
    '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}',
    '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}', '\u{2DEF}', '\u{2DF0}', '\u{2DF1}',
    '\u{2DF2}', '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}',
    '\u{2DF9}', '\u{2DFA}', '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}',
    '\u{A66F}', '\u{A67C}', '\u{A67D}', '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}',
    '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}', '\u{A8E6}', '\u{A8E7}', '\u{A8E8}',
    '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}', '\u{A8EE}', '\u{A8EF}',
    '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}', '\u{AAB8}',
    '\u{AABE}', '\u{AABF}', '\u{AAC1}', '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}',
    '\u{FE24}', '\u{FE25}', '\u{FE26}', '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}',
    '\u{1D187}', '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}', '\u{1D1AD}',
    '\u{1D242}', '\u{1D243}', '\u{1D244}',
];

#[cfg(test)]
mod tests {
    use super::*;
    use helix_view::graphics::Rect;

    #[test]
    fn id_color_encodes_low_24_bits() {
        assert_eq!(image_id_color(0x00_03_E8), Color::Rgb(0x00, 0x03, 0xE8)); // id 1000
        assert_eq!(image_id_color(0xAB_CD_EF), Color::Rgb(0xAB, 0xCD, 0xEF));
    }

    #[test]
    fn placeholder_row_has_one_grapheme_per_column() {
        let row = placeholder_row(0, 4, 1000);
        // Each cell is U+10EEEE + row diacritic + column diacritic => 3 chars per column.
        assert_eq!(row.matches(PLACEHOLDER).count(), 4);
    }

    #[test]
    fn placeholder_cells_occupy_one_column_each() {
        // Render a row of placeholders into a Surface and confirm each lands in its own cell
        // (i.e. the grapheme cluster has display width 1, so columns aren't skipped or merged).
        let area = Rect::new(0, 0, 10, 1);
        let mut surface = Surface::empty(area);
        place_image_rows(&mut surface, 0, 0, 5, 0, 1, 1000);

        for x in 0..5u16 {
            let cell = surface.get(x, 0).expect("cell in bounds");
            assert!(
                cell.symbol.starts_with(PLACEHOLDER),
                "cell {x} should hold a placeholder, got {:?}",
                cell.symbol
            );
            assert_eq!(cell.fg, image_id_color(1000));
        }
        // The 6th cell must be untouched (placeholders didn't bleed past `cols`).
        assert!(!surface
            .get(5, 0)
            .expect("cell in bounds")
            .symbol
            .starts_with(PLACEHOLDER));
    }

    #[test]
    fn high_id_byte_adds_third_diacritic() {
        let low = placeholder_row(0, 1, 0x00_00_01);
        let high = placeholder_row(0, 1, 0x01_00_00_01);
        // The high-byte id carries an extra diacritic per cell.
        assert!(high.chars().count() > low.chars().count());
    }
}
