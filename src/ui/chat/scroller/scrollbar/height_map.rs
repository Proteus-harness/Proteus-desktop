//! Sparse scalar layout facts outlive the resident message window.
//!
//! Unknown runs retain their original per-row estimate. Measuring one row
//! never changes other unknown priors. A compacted aggregate instead lends
//! its retained mass to newly measured rows before redistributing its remainder.
//! The summed tree is the same
//! primitive used by Zed's `crates/gpui/src/elements/list.rs`.
use std::{cmp::Ordering, ops::Range};

use sum_tree::{Bias, Dimension, Item, SeekTarget, SumTree, Summary};

/// Logical accounting for a span and its share of tree nodes. This is a cache
/// budget, not an allocator/RSS measurement. The owner combines chat budgets.
pub(super) const SPAN_BYTES: usize = 128;
pub(super) const DEFAULT_MAX_SPANS: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Unknown,
    Measured,
    Stale,
    Approx,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Span {
    rows: usize,
    pixels: f64,
    kind: Kind,
}

impl Span {
    fn height(self) -> f64 {
        self.pixels / self.rows as f64
    }

    fn split_at(self, rows: usize) -> (Self, Self) {
        let pixels = self.height() * rows as f64;
        (
            Self {
                rows,
                pixels,
                ..self
            },
            Self {
                rows: self.rows - rows,
                pixels: self.pixels - pixels,
                ..self
            },
        )
    }

    fn joins(self, other: Self) -> bool {
        self.kind == other.kind && self.height() == other.height()
    }
}

#[derive(Clone, Default)]
struct Size {
    rows: usize,
    pixels: f64,
    spans: usize,
    measured: usize,
}

impl Summary for Size {
    type Context<'a> = ();
    fn zero(_: ()) -> Self {
        Self::default()
    }
    fn add_summary(&mut self, other: &Self, _: ()) {
        self.rows += other.rows;
        self.pixels += other.pixels;
        self.spans += other.spans;
        self.measured += other.measured;
    }
}

impl Item for Span {
    type Summary = Size;
    fn summary(&self, _: ()) -> Size {
        Size {
            rows: self.rows,
            pixels: self.pixels,
            spans: 1,
            measured: if matches!(self.kind, Kind::Measured | Kind::Stale) {
                self.rows
            } else {
                0
            },
        }
    }
}

#[derive(Clone, Default)]
struct Position {
    rows: usize,
    pixels: f64,
}

impl<'a> Dimension<'a, Size> for Position {
    fn zero(_: ()) -> Self {
        Self::default()
    }
    fn add_summary(&mut self, size: &'a Size, _: ()) {
        self.rows += size.rows;
        self.pixels += size.pixels;
    }
}

struct Row(usize);
impl<'a> SeekTarget<'a, Size, Position> for Row {
    fn cmp(&self, position: &Position, _: ()) -> Ordering {
        Ord::cmp(&self.0, &position.rows)
    }
}
struct Pixel(f64);
impl<'a> SeekTarget<'a, Size, Position> for Pixel {
    fn cmp(&self, position: &Position, _: ()) -> Ordering {
        self.0.total_cmp(&position.pixels)
    }
}

pub(super) struct HeightMap {
    tree: SumTree<Span>,
    prior: f64,
    max_spans: usize,
    revision: u64,
}

impl HeightMap {
    #[cfg(test)]
    pub fn new(total: usize, prior: f64) -> Self {
        Self::with_limit(total, prior, DEFAULT_MAX_SPANS)
    }

    pub fn with_limit(total: usize, prior: f64, max_spans: usize) -> Self {
        assert!(prior.is_finite() && prior > 0.);
        assert!(max_spans > 0);
        let mut this = Self {
            tree: SumTree::new(()),
            prior,
            max_spans,
            revision: 0,
        };
        this.resize(total);
        this
    }

    pub fn total_rows(&self) -> usize {
        self.tree.summary().rows
    }
    pub fn total_height(&self) -> f64 {
        self.tree.summary().pixels
    }
    pub fn span_count(&self) -> usize {
        self.tree.summary().spans
    }
    pub fn measured_rows(&self) -> usize {
        self.tree.summary().measured
    }
    pub fn accounted_bytes(&self) -> usize {
        self.span_count().saturating_mul(SPAN_BYTES)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn prefix(&self, row: usize) -> f64 {
        let row = row.min(self.total_rows());
        let (start, _, span) = self.tree.find((), &Row(row), Bias::Right);
        start.pixels + span.map_or(0., |span| span.height() * (row - start.rows) as f64)
    }

    /// Last measured scalar and whether it is still exact for this layout.
    /// A stale fact remains an estimate but cannot settle a fractional seek.
    /// Coarse aggregates preserve length but do not claim an individual fact.
    pub fn lookup(&self, row: usize) -> Option<(f64, bool)> {
        let (_, _, span) = self.tree.find((), &Row(row), Bias::Right);
        let span = span.filter(|span| matches!(span.kind, Kind::Measured | Kind::Stale))?;
        Some((span.height(), span.kind == Kind::Measured))
    }

    pub fn estimate(&self, row: usize) -> Option<f64> {
        self.tree
            .find((), &Row(row), Bias::Right)
            .2
            .map(|span| span.height())
    }

    /// Locate a global row without materializing its message or widget.
    /// Zero-height runs have no pixel interval and are skipped to the right.
    pub fn target(&self, pixel: f64) -> Option<(usize, f64)> {
        if self.total_rows() == 0 || !pixel.is_finite() {
            return None;
        }
        let height = self.total_height();
        if pixel >= height {
            return Some((self.total_rows(), 0.));
        }
        let (start, _, span) = self.tree.find((), &Pixel(pixel.max(0.)), Bias::Right);
        let span = span?;
        let ordinal = ((pixel.max(0.) - start.pixels) / span.height()).clamp(0., span.rows as f64);
        let offset = (ordinal.floor() as usize).min(span.rows - 1);
        Some((start.rows + offset, (ordinal - offset as f64).clamp(0., 1.)))
    }

    /// Standalone updates protect their changed row. A resident list supplies
    /// its whole window to `set_limited` so compaction preserves its geometry.
    #[cfg(test)]
    pub fn set(&mut self, row: usize, height: f64) -> bool {
        self.set_limited(row, height, self.max_spans, row..row.saturating_add(1))
    }

    pub fn set_limited(
        &mut self,
        row: usize,
        height: f64,
        max_spans: usize,
        protected: Range<usize>,
    ) -> bool {
        if row >= self.total_rows() || !height.is_finite() || height < 0. {
            return false;
        }
        let replacement = Span {
            rows: 1,
            pixels: height,
            kind: Kind::Measured,
        };
        if self
            .span_at(row)
            .is_some_and(|span| span.joins(replacement))
        {
            return false;
        }
        // A point update creates at most two new boundaries. Coarsen cold
        // regions before admitting them, retaining their aggregate lengths.
        if self.replacement_spans(row, replacement) > max_spans {
            self.compact(protected, max_spans.saturating_sub(2).max(1));
        }
        if self.replacement_spans(row, replacement) > max_spans {
            return false;
        }
        let (start, _, span) = self.tree.find((), &Row(row), Bias::Right);
        let span = *span.unwrap();
        if span.kind == Kind::Approx && span.rows > 1 {
            // Compaction discarded individual heights, not their summed mass.
            // Refining one row must not subtract the coarse average as though
            // it were an old measurement. Spend its fact from this aggregate
            // and distribute the remaining pixels across its other rows.
            // A singleton has no remaining approximation and uses the actual
            // value, so genuine content changes are not hidden as a debt.
            let before_rows = row - start.rows;
            let after_rows = span.rows - before_rows - 1;
            let remaining = (span.pixels - height).max(0.);
            let before_pixels = remaining * before_rows as f64 / (span.rows - 1) as f64;
            let mut refined = SumTree::new(());
            if before_rows > 0 {
                refined.push(
                    Span {
                        rows: before_rows,
                        pixels: before_pixels,
                        kind: Kind::Approx,
                    },
                    (),
                );
            }
            refined.push(replacement, ());
            if after_rows > 0 {
                refined.push(
                    Span {
                        rows: after_rows,
                        pixels: remaining - before_pixels,
                        kind: Kind::Approx,
                    },
                    (),
                );
            }
            self.replace(start.rows..start.rows + span.rows, refined);
        } else {
            self.replace(row..row + 1, SumTree::from_item(replacement, ()));
        }
        true
    }

    fn replacement_spans(&self, row: usize, replacement: Span) -> usize {
        let (start, _, span) = self.tree.find((), &Row(row), Bias::Right);
        let span = *span.unwrap();
        let mut count = self.span_count()
            + usize::from(row > start.rows)
            + usize::from(row + 1 < start.rows + span.rows);
        if row == start.rows
            && row > 0
            && self
                .span_at(row - 1)
                .is_some_and(|span| span.joins(replacement))
        {
            count -= 1;
        }
        if row + 1 == start.rows + span.rows
            && self
                .span_at(row + 1)
                .is_some_and(|span| span.joins(replacement))
        {
            count -= 1;
        }
        count
    }

    pub fn invalidate(&mut self, rows: Range<usize>) {
        let rows = rows.start.min(self.total_rows())..rows.end.min(self.total_rows());
        if rows.is_empty() {
            return;
        }
        // Freshness may split two run boundaries. At the limit, making all
        // facts stale preserves every estimate without growing metadata.
        if self.span_count().saturating_add(2) > self.max_spans {
            self.invalidate_all();
            return;
        }
        let (left, rest) = split(&self.tree, rows.start);
        let (middle, right) = split(&rest, rows.len());
        if !middle.iter().any(|span| span.kind == Kind::Measured) {
            return;
        }
        let mut stale = SumTree::new(());
        for span in middle.iter() {
            stale = join(
                stale,
                SumTree::from_item(
                    Span {
                        kind: if span.kind == Kind::Measured {
                            Kind::Stale
                        } else {
                            span.kind
                        },
                        ..*span
                    },
                    (),
                ),
            );
        }
        self.assign(join(join(left, stale), right));
    }

    pub fn invalidate_all(&mut self) {
        if !self.tree.iter().any(|span| span.kind == Kind::Measured) {
            return;
        }
        let mut stale = SumTree::new(());
        for span in self.tree.iter() {
            stale = join(
                stale,
                SumTree::from_item(
                    Span {
                        kind: if span.kind == Kind::Measured {
                            Kind::Stale
                        } else {
                            span.kind
                        },
                        ..*span
                    },
                    (),
                ),
            );
        }
        self.assign(stale);
    }

    /// Collapse cold regions to coarse spans, preserving their aggregate
    /// pixel lengths. Protected rows retain individual facts. If the protected
    /// region alone exceeds the requested limit this is best-effort; the owner
    /// must inspect the remaining count before admitting more measurements.
    pub fn compact(&mut self, protected: Range<usize>, max_spans: usize) -> bool {
        if self.span_count() <= max_spans.max(1) {
            return false;
        }
        let total = self.total_rows();
        let start = protected.start.min(total);
        let end = protected.end.max(start).min(total);
        if start == end {
            self.assign(coarse(self.tree.clone()));
            return true;
        }
        let (before, rest) = split(&self.tree, start);
        let (middle, after) = split(&rest, end - start);
        let next = join(join(coarse(before), middle), coarse(after));
        let changed = next.summary().spans < self.span_count();
        if changed {
            self.assign(next);
        }
        changed
    }

    /// Retain only the immutable prefix across a canonical revision, replacing
    /// the changed suffix by its stable unknown-row prior.
    pub fn rebase(&mut self, keep_prefix: usize, total: usize) {
        let keep = keep_prefix.min(total).min(self.total_rows());
        if keep < self.total_rows() {
            self.assign(split(&self.tree, keep).0);
        }
        self.resize(total);
    }

    /// Appending/removing rows preserves surviving facts. One new unknown
    /// suffix and an invalidation boundary may use two structural reserve
    /// spans beyond the measurement limit; no measurement can grow it further.
    pub fn resize(&mut self, total: usize) {
        match Ord::cmp(&total, &self.total_rows()) {
            Ordering::Less => self.assign(split(&self.tree, total).0),
            Ordering::Greater => {
                let rows = total - self.total_rows();
                self.assign(join(
                    self.tree.clone(),
                    SumTree::from_item(
                        Span {
                            rows,
                            pixels: self.prior * rows as f64,
                            kind: Kind::Unknown,
                        },
                        (),
                    ),
                ));
            }
            Ordering::Equal => {}
        }
    }

    pub fn reset(&mut self, total: usize, prior: f64) {
        assert!(prior.is_finite() && prior > 0.);
        if self.prior == prior
            && self.total_rows() == total
            && self.tree.iter().all(|span| span.kind == Kind::Unknown)
        {
            return;
        }
        self.assign(SumTree::new(()));
        self.prior = prior;
        self.resize(total);
    }

    fn span_at(&self, row: usize) -> Option<Span> {
        self.tree.find((), &Row(row), Bias::Right).2.copied()
    }

    fn replace(&mut self, rows: Range<usize>, replacement: SumTree<Span>) {
        let (left, rest) = split(&self.tree, rows.start);
        let (_, right) = split(&rest, rows.len());
        self.assign(join(join(left, replacement), right));
    }

    fn assign(&mut self, tree: SumTree<Span>) {
        self.tree = tree;
        self.revision = self.revision.wrapping_add(1);
    }
}

fn split(tree: &SumTree<Span>, row: usize) -> (SumTree<Span>, SumTree<Span>) {
    let row = row.min(tree.summary().rows);
    let mut cursor = tree.cursor::<Position>(());
    let mut left = cursor.slice(&Row(row), Bias::Right);
    let mut right = SumTree::new(());
    if let Some(span) = cursor.item().copied() {
        let offset = row - cursor.start().rows;
        if offset > 0 {
            let (a, b) = span.split_at(offset);
            left.push(a, ());
            right.push(b, ());
            cursor.next();
        }
    }
    right.append(cursor.suffix(), ());
    (left, right)
}

fn join(mut left: SumTree<Span>, mut right: SumTree<Span>) -> SumTree<Span> {
    if let Some((a, b)) = left.last().copied().zip(right.first().copied())
        && a.joins(b)
    {
        left.update_last(
            |span| {
                span.rows += b.rows;
                span.pixels += b.pixels;
            },
            (),
        );
        right = split(&right, b.rows).1;
    }
    left.append(right, ());
    left
}

fn coarse(tree: SumTree<Span>) -> SumTree<Span> {
    if tree.summary().spans <= 1 {
        return tree;
    }
    SumTree::from_item(
        Span {
            rows: tree.summary().rows,
            pixels: tree.summary().pixels,
            kind: Kind::Approx,
        },
        (),
    )
}

#[cfg(test)]
#[path = "height_map/tests.rs"]
mod tests;
