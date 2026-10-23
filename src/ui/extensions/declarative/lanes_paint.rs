// Адаптировано из Zed: crates/git_ui/src/git_graph.rs
//! Painting of one row of the lane column: lines from the row edges to its
//! middle and the ring of the row's dot.

use crate::declarative::lanes::{Row, Segment};
use gpui_kit::*;

/// Width of one lane.
pub(super) const LANE: f32 = 14.;
/// Room before the first lane.
pub(super) const LEFT: f32 = 4.;
const RADIUS: f32 = 4.;
const LINE: f32 = 1.5;

/// Radix step 9 hues that read on dark and light panels alike.
const PALETTE: [u32; crate::declarative::lanes::COLORS] = [
    0x0090ff, 0xf76b15, 0xd6409f, 0x46a758, 0x8e4ec6, 0x00a2c7, 0xe5a000, 0xe54d2e,
];

pub(super) fn color(index: usize) -> Hsla {
    rgb(PALETTE[index % PALETTE.len()]).into()
}

/// Paints `row` into `bounds`, the lane column of the row.
pub(super) fn paint(bounds: Bounds<Pixels>, row: &Row, window: &mut Window) {
    let x = |lane: usize| bounds.origin.x + px(LEFT + lane as f32 * LANE + LANE / 2.);
    let (top, bottom) = (bounds.origin.y, bounds.bottom());
    let middle = bounds.center().y;
    let geometry = Geometry {
        dot: (x(row.lane), middle),
        curve: (px(LANE / 3.), bounds.size.height / 3.),
    };
    for segment in &row.up {
        let path = geometry.line(
            point(x(segment.from), top),
            point(x(segment.to), middle),
            segment.to == row.lane,
            false,
        );
        stroke(path, segment, window);
    }
    for segment in &row.down {
        let path = geometry.line(
            point(x(segment.from), middle),
            point(x(segment.to), bottom),
            segment.from == row.lane,
            true,
        );
        stroke(path, segment, window);
    }
    ring(geometry.dot.0, middle, color(row.color), window);
}

struct Geometry {
    dot: (Pixels, Pixels),
    /// Largest width and height of the bend into or out of the dot.
    curve: (Pixels, Pixels),
}

impl Geometry {
    /// A line from `start` to `end`. One that touches the dot stops at its
    /// ring and bends into it sideways; a line between two lanes elsewhere
    /// is a smooth step.
    fn line(
        &self,
        start: Point<Pixels>,
        end: Point<Pixels>,
        dot: bool,
        leaves: bool,
    ) -> PathBuilder {
        let mut path = PathBuilder::stroke(px(LINE));
        let gap = px(RADIUS + LINE / 2.);
        if start.x == end.x {
            let (start, end) = match (dot, leaves) {
                (true, true) => (point(start.x, start.y + gap), end),
                (true, false) => (start, point(end.x, end.y - gap)),
                (false, _) => (start, end),
            };
            path.move_to(start);
            path.line_to(end);
            return path;
        }
        let right = end.x > start.x;
        let sign = if right { 1. } else { -1. };
        if !dot {
            let half = (end.y - start.y) / 2.;
            path.move_to(start);
            path.cubic_bezier_to(
                end,
                point(start.x, start.y + half),
                point(end.x, end.y - half),
            );
            return path;
        }
        let (width, height) = self.curve;
        if leaves {
            // From the side of the dot across to the lane, then down.
            let from = point(start.x + gap * sign, start.y);
            let width = width.min((end.x - from.x).abs());
            let height = height.min(end.y - start.y);
            let bend = point(end.x - width * sign, start.y);
            path.move_to(from);
            path.line_to(bend);
            path.move_to(bend);
            path.curve_to(point(end.x, start.y + height), point(end.x, start.y));
            path.move_to(point(end.x, start.y + height));
            path.line_to(end);
        } else {
            // Down the lane, then across into the side of the dot.
            let to = point(end.x - gap * sign, end.y);
            let width = width.min((to.x - start.x).abs());
            let height = height.min(end.y - start.y);
            let bend = point(start.x, end.y - height);
            path.move_to(start);
            path.line_to(bend);
            path.move_to(bend);
            path.curve_to(point(start.x + width * sign, end.y), point(start.x, end.y));
            path.move_to(point(start.x + width * sign, end.y));
            path.line_to(to);
        }
        path
    }
}

fn stroke(path: PathBuilder, segment: &Segment, window: &mut Window) {
    if let Ok(path) = path.build() {
        window.paint_path(path, color(segment.color));
    }
}

/// The dot: a ring in the row color, open in the middle.
fn ring(x: Pixels, y: Pixels, color: Hsla, window: &mut Window) {
    let radius = px(RADIUS - LINE / 2.);
    let mut path = PathBuilder::stroke(px(LINE));
    path.move_to(point(x + radius, y));
    path.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(x - radius, y),
    );
    path.arc_to(
        point(radius, radius),
        px(0.),
        false,
        true,
        point(x + radius, y),
    );
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}
