//! The static Proteus pixel logo, drawn in native cells.

use crate::ui::prefs;
use gpui_kit::*;

const COLUMNS: f32 = 12.;
const ROWS: f32 = 10.;

/// Runs of one color: column, row, width in cells.
const CELLS: [(u8, u8, u8, u32); 33] = [
    (0, 0, 2, 0xed1c24),
    (2, 0, 2, 0xffa3b1),
    (4, 0, 3, 0x6f3198),
    (7, 0, 3, 0xff7e00),
    (10, 0, 2, 0x2f3699),
    (0, 1, 3, 0xed1c24),
    (3, 1, 2, 0xffa3b1),
    (5, 1, 3, 0x6f3198),
    (8, 1, 3, 0xff7e00),
    (11, 1, 1, 0x2f3699),
    (2, 2, 1, 0xed1c24),
    (3, 2, 1, 0xffa3b1),
    (8, 2, 1, 0xff7e00),
    (9, 2, 1, 0x99d9ea),
    (2, 3, 1, 0xed1c24),
    (3, 3, 1, 0xffa3b1),
    (8, 3, 2, 0x99d9ea),
    (2, 4, 2, 0xffa3b1),
    (8, 4, 2, 0x99d9ea),
    (2, 5, 2, 0x22b14c),
    (8, 5, 2, 0x99d9ea),
    (2, 6, 2, 0x22b14c),
    (8, 6, 1, 0xffc20e),
    (9, 6, 1, 0x99d9ea),
    (2, 7, 1, 0x22b14c),
    (3, 7, 1, 0x990030),
    (8, 7, 1, 0xffc20e),
    (9, 7, 1, 0x99d9ea),
    (2, 8, 1, 0x22b14c),
    (3, 8, 1, 0x990030),
    (8, 8, 2, 0xffc20e),
    (2, 9, 2, 0x990030),
    (8, 9, 2, 0xffc20e),
];

/// The logo `cell` wide per pixel. Activity belongs to chat status controls,
/// rather than an unrelated animation on the application's menu button.
pub(super) fn logo(cell: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let unit = bounds.size.width / COLUMNS;
            for &(x, y, width, color) in &CELLS {
                let origin = bounds.origin + point(unit * f32::from(x), unit * f32::from(y));
                window.paint_quad(fill(
                    Bounds::new(origin, size(unit * f32::from(width), unit)),
                    rgb(color),
                ));
            }
        },
    )
    .flex_none()
    .w(prefs::scaled(cell * COLUMNS))
    .h(prefs::scaled(cell * ROWS))
}
