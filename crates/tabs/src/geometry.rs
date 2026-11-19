//! Where panes are on screen and resizing of splits. Dividers are left
//! out unless the caller gives their thickness: they are thin next to the
//! panes.

use super::layout::TabLayout;
use super::size::{Size, extents, set_extents};
use super::tree::{Axis, Edge, Node, PaneId};

/// Part of the window, in pixels or as fractions of it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

const EPSILON: f32 = 1e-4;

/// Window size in pixels the arrangement is read at when only it matters.
const REFERENCE: (f32, f32) = (1600., 1000.);

/// Share of a pane next to its edge where a dropped tab splits the pane
/// rather than joining it.
const EDGE_ZONE: f32 = 0.3;

impl Rect {
    fn right(self) -> f32 {
        self.x + self.w
    }

    fn bottom(self) -> f32 {
        self.y + self.h
    }
}

/// Edge of a pane a tab dropped at `(x, y)` (fractions of the pane) splits
/// off to; `None` in the middle, where it joins the pane.
pub fn drop_edge(x: f32, y: f32) -> Option<Edge> {
    [
        (x, Edge::Left),
        (1. - x, Edge::Right),
        (y, Edge::Top),
        (1. - y, Edge::Bottom),
    ]
    .into_iter()
    .filter(|(distance, _)| *distance < EDGE_ZONE)
    .min_by(|a, b| a.0.total_cmp(&b.0))
    .map(|(_, edge)| edge)
}

fn place(node: &Node, rect: Rect, gap: f32, out: &mut Vec<(PaneId, Rect)>) {
    match node {
        Node::Pane(pane) => out.push((pane.id, rect)),
        Node::Split(split) => {
            let gaps = gap * split.children.len().saturating_sub(1) as f32;
            let total = match split.axis {
                Axis::Row => rect.w,
                Axis::Column => rect.h,
            } - gaps;
            let mut offset = 0.;
            for (child, len) in split.children.iter().zip(extents(&split.children, total)) {
                let part = match split.axis {
                    Axis::Row => Rect {
                        x: rect.x + offset,
                        w: len,
                        ..rect
                    },
                    Axis::Column => Rect {
                        y: rect.y + offset,
                        h: len,
                        ..rect
                    },
                };
                offset += len + gap;
                place(&child.node, part, gap, out);
            }
        }
    }
}

impl TabLayout {
    /// Every pane with its place in pixels when the whole tree is
    /// `width` × `height`, in reading order.
    pub fn rects_in(&self, width: f32, height: f32) -> Vec<(PaneId, Rect)> {
        self.rects_spaced(width, height, 0.)
    }

    /// Like `rects_in`, with `gap` pixels taken by every divider.
    pub fn rects_spaced(&self, width: f32, height: f32, gap: f32) -> Vec<(PaneId, Rect)> {
        let mut out = Vec::new();
        let whole = Rect {
            x: 0.,
            y: 0.,
            w: width,
            h: height,
        };
        place(&self.root, whole, gap, &mut out);
        out
    }

    /// Every pane with its place as fractions of the tree, for a window of
    /// a typical size: fixed panes make the arrangement depend on it.
    pub fn rects(&self) -> Vec<(PaneId, Rect)> {
        let (width, height) = REFERENCE;
        let fraction = |rect: Rect| Rect {
            x: rect.x / width,
            y: rect.y / height,
            w: rect.w / width,
            h: rect.h / height,
        };
        self.rects_in(width, height)
            .into_iter()
            .map(|(id, rect)| (id, fraction(rect)))
            .collect()
    }

    pub fn rect(&self, pane: PaneId) -> Option<Rect> {
        self.rects()
            .into_iter()
            .find(|(id, _)| *id == pane)
            .map(|(_, rect)| rect)
    }

    /// The pane right beyond `edge` of `pane`: the closest one along the
    /// way, then the one sharing the longest border.
    pub fn neighbor(&self, pane: PaneId, edge: Edge) -> Option<PaneId> {
        Self::neighbor_in(&self.rects(), pane, edge)
    }

    pub(super) fn neighbor_in(
        rects: &[(PaneId, Rect)],
        pane: PaneId,
        edge: Edge,
    ) -> Option<PaneId> {
        let from = rects.iter().find(|(id, _)| *id == pane)?.1;
        let overlap = |a0: f32, a1: f32, b0: f32, b1: f32| a1.min(b1) - a0.max(b0);
        rects
            .iter()
            .copied()
            .filter(|(id, rect)| *id != pane && rect.w > 0. && rect.h > 0.)
            .filter_map(|(id, to)| {
                let (gap, shared) = match edge {
                    Edge::Right => (
                        to.x - from.right(),
                        overlap(from.y, from.bottom(), to.y, to.bottom()),
                    ),
                    Edge::Left => (
                        from.x - to.right(),
                        overlap(from.y, from.bottom(), to.y, to.bottom()),
                    ),
                    Edge::Bottom => (
                        to.y - from.bottom(),
                        overlap(from.x, from.right(), to.x, to.right()),
                    ),
                    Edge::Top => (
                        from.y - to.bottom(),
                        overlap(from.x, from.right(), to.x, to.right()),
                    ),
                };
                (gap > -EPSILON && shared > EPSILON).then_some((id, gap, shared))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1).then(b.2.total_cmp(&a.2)))
            .map(|(id, _, _)| id)
    }

    /// Moves the divider after child `index` of the split at `path` to
    /// `at` pixels into the split, which has `total` pixels for its
    /// children. Each pane keeps at least `min` pixels.
    pub fn resize(&mut self, path: &[usize], index: usize, at: f32, total: f32, min: f32) {
        let Some(split) = self.root.split_at_mut(path) else {
            return;
        };
        if index + 1 >= split.children.len() {
            return;
        }
        let mut lengths = extents(&split.children, total);
        let start: f32 = lengths[..index].iter().sum();
        let before = lengths[index];
        let requested = at - (start + before);
        let pair = before + lengths[index + 1];
        let min_a = split.children[index].node.min_panes(split.axis) as f32 * min;
        let min_b = split.children[index + 1].node.min_panes(split.axis) as f32 * min;
        let enough_room = min_a + min_b <= pair;
        let at = if enough_room {
            at.clamp(start + min_a, start + pair - min_b)
        } else {
            start + pair * min_a / (min_a + min_b)
        };
        lengths[index] = at - start;
        lengths[index + 1] = pair - lengths[index];
        if set_extents(&mut split.children, &lengths, &[index, index + 1]) {
            return;
        }

        // All flexible siblings may have lost their pixels to these two
        // fixed panes. Their positive weights still describe how to divide
        // room after the window grows; there is no reason to replace them.
        // When clamping, the divider may stop short of the pointer. It
        // must not jump past it, reverse direction, or move on a still drag.
        let applied = lengths[index] - before;
        let follows = if requested > EPSILON {
            applied > 0. && applied <= requested
        } else {
            requested < -EPSILON && applied < 0. && applied >= requested
        };
        if enough_room
            && follows
            && split.children.iter().enumerate().all(|(i, child)| {
                if i == index || i == index + 1 {
                    child.size.is_fixed() && lengths[i].is_finite() && lengths[i] > 0.
                } else {
                    !child.size.is_fixed() && lengths[i] == 0.
                }
            })
        {
            split.children[index].size = Size::Fixed(lengths[index]);
            split.children[index + 1].size = Size::Fixed(lengths[index + 1]);
        }
    }

    /// Gives every weighted child of the split at `path` the same share;
    /// fixed ones keep their pixels.
    pub fn reset(&mut self, path: &[usize]) {
        if let Some(split) = self.root.split_at_mut(path) {
            for child in &mut split.children {
                if !child.size.is_fixed() {
                    child.size = Size::Weight(1.);
                }
            }
        }
    }

    /// Sizes of the children of the split at `path`.
    pub fn sizes(&self, path: &[usize]) -> Option<Vec<Size>> {
        let split = self.root.split_at(path)?;
        Some(split.children.iter().map(|child| child.size).collect())
    }

    /// Puts back sizes taken by [`Self::sizes`]; ignored when the split
    /// changed meanwhile.
    pub fn set_sizes(&mut self, path: &[usize], sizes: &[Size]) {
        let Some(split) = self.root.split_at_mut(path) else {
            return;
        };
        let fits = split.children.len() == sizes.len()
            && sizes.iter().all(|size| size.is_valid())
            && sizes.iter().any(|size| !size.is_fixed())
            && split
                .children
                .iter()
                .zip(sizes)
                .all(|(child, size)| child.size.is_fixed() == size.is_fixed());
        if !fits {
            return;
        }
        for (child, size) in split.children.iter_mut().zip(sizes) {
            child.size = *size;
        }
    }
}
