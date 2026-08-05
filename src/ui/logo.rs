use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

// The thrown rose — chiba's mark, where tuxedo has the bowtie.
//
// Drawn at *half-block* resolution: the design grid is 16×10 square pixels,
// and each terminal cell carries two vertically-stacked pixels via U+2580
// (upper half block), with the lower pixel showing as that cell's background.
// A terminal cell is roughly 1:2, so a half-cell pixel comes out square —
// twice the vertical detail of the full-block approach for the same 5 rows
// on screen.
//
// The bloom takes priority-A red and the stem/leaf take the theme accent, so
// chiba reuses tuxedo's two logo colours in a shape you can't confuse with it.
// No third theme field, and it works in every palette including Terminal's
// plain Red/Cyan.
pub const WIDTH: u16 = 16;
pub const HEIGHT: u16 = 5;

const UPPER: &str = "▀";
const FULL: &str = "█";
const LOWER: &str = "▄";

/// 16 columns × 10 pixel-rows. `k` = bloom (priority-A red), `b` = stem and
/// leaf (theme accent), `.` = empty. Deliberately asymmetric: a symmetric
/// blob with interior gaps reads as a *face* at this size — an angled stem
/// reads as a rose in flight.
const GRID: [&[u8; 16]; 10] = [
    b"..kkkk..........",
    b".kkkkkk.........",
    b".kkkkkk.........",
    b"..kkkk.b........",
    b"...kk...b.......",
    b".........b......",
    b"......bbb.b.....",
    b"...........b....",
    b"............b...",
    b".............b..",
];

/// Colour of a design pixel, or `None` for empty.
fn pixel(theme: &Theme, c: u8) -> Option<Color> {
    match c {
        b'k' => Some(theme.pri_a),
        b'b' => Some(theme.accent),
        _ => None,
    }
}

pub fn centered_lines(theme: &Theme, inner_width: u16) -> Vec<Line<'static>> {
    let pad_w = inner_width.saturating_sub(WIDTH) / 2;
    let pad = " ".repeat(pad_w as usize);

    GRID.chunks(2)
        .map(|pair| {
            let (top, bottom) = (pair[0], pair[1]);
            let mut spans: Vec<Span<'static>> = Vec::with_capacity(WIDTH as usize + 1);
            spans.push(Span::raw(pad.clone()));
            for col in 0..WIDTH as usize {
                spans.push(match (pixel(theme, top[col]), pixel(theme, bottom[col])) {
                    (None, None) => Span::raw(" "),
                    (Some(t), None) => Span::styled(UPPER, Style::default().fg(t)),
                    (None, Some(b)) => Span::styled(LOWER, Style::default().fg(b)),
                    (Some(t), Some(b)) if t == b => Span::styled(FULL, Style::default().fg(t)),
                    // Two different colours in one cell: top half in the
                    // foreground, bottom half as the cell background.
                    (Some(t), Some(b)) => Span::styled(UPPER, Style::default().fg(t).bg(b)),
                });
            }
            Line::from(spans)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> &'static Theme {
        crate::theme::all()[0]
    }

    fn cells(line: &Line<'static>) -> usize {
        line.spans.iter().map(|s| s.content.chars().count()).sum()
    }

    #[test]
    fn grid_is_well_formed() {
        assert_eq!(GRID.len() % 2, 0, "pixel rows must pair into terminal rows");
        assert_eq!(GRID.len() / 2, HEIGHT as usize);
        assert!(GRID.iter().all(|r| r.len() == WIDTH as usize));
        assert!(
            GRID.iter()
                .flat_map(|r| r.iter())
                .all(|c| matches!(c, b'k' | b'b' | b'.')),
            "only bloom, stem, and empty pixels are defined",
        );
    }

    #[test]
    fn renders_height_rows_of_exactly_width_cells() {
        let lines = centered_lines(theme(), WIDTH);
        assert_eq!(lines.len(), HEIGHT as usize);
        for line in &lines {
            assert_eq!(cells(line), WIDTH as usize, "each row must be WIDTH cells");
        }
    }

    #[test]
    fn bloom_is_red_and_stem_is_accent() {
        let t = theme();
        assert_eq!(pixel(t, GRID[0][2]), Some(t.pri_a), "grid row 0 is bloom");
        assert_eq!(pixel(t, GRID[8][12]), Some(t.accent), "grid row 8 is stem");
        assert_eq!(pixel(t, b'.'), None);
    }

    #[test]
    fn centering_pads_without_overflowing_a_narrow_panel() {
        assert_eq!(cells(&centered_lines(theme(), 60)[0]), 22 + WIDTH as usize);
        // Narrower than the logo: no padding rather than a panic or underflow.
        let narrow = centered_lines(theme(), 4);
        assert_eq!(narrow.len(), HEIGHT as usize);
        assert_eq!(cells(&narrow[0]), WIDTH as usize);
    }
}
