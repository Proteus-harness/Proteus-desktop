//! Cursor feedback follows already shaped visual rows. Selection hitboxes stay
//! independent so dragging through empty space can continue a selection.
use gpui::{Bounds, Pixels, TextAlign, TextLayout, point, size};

/// Empty rows and invisible margins are not text cursor targets. Reused by
/// rich/plain text and read-only editors without shaping or copying source.
pub(crate) fn clipped_text_row(
    row: Bounds<Pixels>,
    mask: Bounds<Pixels>,
) -> Option<Bounds<Pixels>> {
    let row = row.intersect(&mask);
    (row.size.width > Pixels::ZERO && row.size.height > Pixels::ZERO).then_some(row)
}

/// Actual painted row extents for cursor feedback. Unlike selection geometry,
/// these boxes never extend through a newline or into the blank row margin.
/// Wrapped glyph boundaries are already shaped by GPUI, so this is O(rows),
/// with no source clone or second shaping pass.
pub(crate) fn text_cursor_bounds(
    text_layout: &TextLayout,
    align: TextAlign,
    mask: Bounds<Pixels>,
) -> smallvec::SmallVec<[Bounds<Pixels>; 1]> {
    let bounds = text_layout.bounds();
    let line_height = text_layout.line_height();
    let mut result = smallvec::SmallVec::new();
    let mut y = bounds.top();
    for line in text_layout.line_layouts() {
        let layout = &line.unwrapped_layout;
        let mut start_x = Pixels::ZERO;
        let row_ends = line
            .wrap_boundaries
            .iter()
            .map(|boundary| {
                layout.runs[boundary.run_ix].glyphs[boundary.glyph_ix]
                    .position
                    .x
            })
            .chain([layout.width]);
        for end_x in row_ends {
            let width = end_x - start_x;
            let left = bounds.left() + aligned_row_left(align, bounds.size.width, width);
            if let Some(row) =
                clipped_text_row(Bounds::new(point(left, y), size(width, line_height)), mask)
            {
                result.push(row);
            }
            y += line_height;
            start_x = end_x;
        }
    }
    result
}

pub(crate) fn aligned_row_left(align: TextAlign, align_width: Pixels, width: Pixels) -> Pixels {
    match align {
        TextAlign::Left => Pixels::ZERO,
        TextAlign::Center => (align_width - width) / 2.,
        TextAlign::Right => align_width - width,
    }
}
