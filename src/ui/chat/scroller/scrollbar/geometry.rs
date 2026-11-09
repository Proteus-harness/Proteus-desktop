//! Bounded pixel geometry. Wheel position follows the current committed pixel
//! estimate; only an explicit seek pins a requested track position.
use super::HistoryRange;
use gpui_kit::{ListOffset, ListState, Pixels, px};

#[derive(Clone, Copy, Default, Debug)]
struct Knot {
    pixel: f64,
    track: f64,
}

#[derive(Clone, Copy, Default, Debug)]
struct Mapping {
    left: Knot,
    pivot: Knot,
    right: Knot,
}

impl Mapping {
    fn new(left: Knot, right: Knot, pivot: Option<Knot>) -> Self {
        let pivot = pivot
            .filter(|p| {
                p.pixel > left.pixel
                    && p.pixel < right.pixel
                    && p.track >= left.track
                    && p.track <= right.track
            })
            .unwrap_or(left);
        Self { left, pivot, right }
    }

    fn convert(&self, value: f64, inverse: bool) -> f64 {
        let coordinate = |k: Knot| if inverse { k.track } else { k.pixel };
        let output = |k: Knot| if inverse { k.pixel } else { k.track };
        let (a, b) = if value < coordinate(self.pivot) {
            (self.left, self.pivot)
        } else {
            (self.pivot, self.right)
        };
        let span = coordinate(b) - coordinate(a);
        if span <= 0. {
            return output(a);
        }
        output(a) + (value - coordinate(a)) / span * (output(b) - output(a))
    }
}

#[derive(Clone, Copy)]
struct Anchor {
    row: usize,
    offset: Pixels,
    track: Pixels,
}

#[derive(Default)]
pub(super) struct Geometry {
    range: Option<HistoryRange>,
    /// Last committed scalar facts remain useful estimates while a renderer
    /// remeasures. Freshness separately gates exact fractional positioning.
    heights: Vec<Option<Pixels>>,
    stale: Vec<bool>,
    facts_revision: u64,
    /// Prefix sums of committed heights. Offset reads never rescan the page.
    prefix: Vec<f64>,
    prefix_dirty: bool,
    measured_sum: Pixels,
    measured_count: usize,
    estimate: Pixels,
    viewport: Pixels,
    tail: Pixels,
    padding: Pixels,
    max: Pixels,
    mapping: Mapping,
    anchor: Option<Anchor>,
    invalidated: bool,
}

impl Geometry {
    #[cfg(debug_assertions)]
    pub fn diagnostic_mapping(&self) -> [[f64; 2]; 3] {
        [self.mapping.left, self.mapping.pivot, self.mapping.right].map(|k| [k.pixel, k.track])
    }

    #[cfg(any(debug_assertions, test))]
    pub fn facts_revision(&self) -> u64 {
        self.facts_revision
    }

    pub fn represents(&self, range: HistoryRange) -> bool {
        self.range == Some(range)
    }

    /// A censored viewport can contain only the middle of one huge reply.
    /// Ordinary measurements elsewhere in this resident page are a better
    /// sample for unknown rows and need no additional rendering.
    pub fn ordinary_height(&self, viewport: Pixels) -> Option<Pixels> {
        let mut sum = px(0.);
        let mut count = 0;
        for height in self.heights.iter().flatten() {
            if *height >= px(0.) && *height <= viewport * 2. {
                sum += *height;
                count += 1;
            }
        }
        // Zeros participate in a mixed sample, but an entirely folded page
        // says nothing about the unseen prefix. Keep its ordinary-row prior.
        (count > 0 && sum > px(0.)).then(|| sum / count as f32)
    }

    /// Count each measured reply once, rather than repeating its full height
    /// over every unknown row. New facts refine extent and position together.
    pub fn known_correction(&self, estimate: Pixels) -> Option<Pixels> {
        let range = self.range?;
        (self.measured_count > 0 || range.rows == 0).then(|| {
            self.measured_sum - estimate * self.measured_count as f32 + self.tail + self.padding
        })
    }

    pub fn refresh(
        &mut self,
        list: &ListState,
        range: HistoryRange,
        estimate: Pixels,
        max: Pixels,
        padding: Pixels,
        measurements: impl Iterator<Item = (usize, Pixels)>,
    ) -> bool {
        let mut facts_changed = false;
        let viewport = list.viewport_bounds().size.height;
        let mut changed = std::mem::take(&mut self.invalidated)
            || self.range != Some(range)
            || self.estimate != estimate
            || self.viewport != viewport
            || self.max != max
            || self.padding != padding;
        if self.range != Some(range) {
            facts_changed = true;
            let old = self.range;
            let stale = (range.start..range.start + range.rows)
                .map(|row| {
                    old.and_then(|old| row.checked_sub(old.start))
                        .and_then(|ix| self.stale.get(ix))
                        .copied()
                        .unwrap_or(false)
                })
                .collect();
            self.heights = (range.start..range.start + range.rows)
                .map(|row| {
                    old.and_then(|old| row.checked_sub(old.start))
                        .and_then(|ix| self.heights.get(ix))
                        .copied()
                        .flatten()
                })
                .collect();
            self.stale = stale;
            self.range = Some(range);
            self.tail = px(0.);
            self.prefix_dirty = true;
        }
        self.prefix_dirty |= self.estimate != estimate;
        self.estimate = estimate;
        self.viewport = viewport;
        self.max = max;
        self.padding = padding;
        // Consume only a completed layout's batch, including leading overdraw.
        // Item bounds deliberately hide that overdraw above logical_scroll_top.
        for (ix, height) in measurements {
            if ix == range.rows {
                facts_changed |= self.tail != height;
                changed |= self.tail != height;
                self.tail = height;
            } else if let Some(cached) = self.heights.get_mut(ix) {
                if *cached != Some(height) {
                    #[cfg(debug_assertions)]
                    super::scroll_trace::record(|| {
                        serde_json::json!({"phase":"height", "row":range.start+ix,
                            "before":cached.map(f32::from), "after":f32::from(height)})
                    });
                    *cached = Some(height);
                    self.prefix_dirty = true;
                    facts_changed = true;
                    changed = true;
                }
                if let Some(stale) = self.stale.get_mut(ix) {
                    changed |= std::mem::replace(stale, false);
                }
            }
        }
        if facts_changed {
            self.facts_revision = self.facts_revision.wrapping_add(1);
        }
        if changed {
            self.rebuild();
        }
        changed
    }

    fn height(&self, ix: usize) -> Pixels {
        self.heights[ix].unwrap_or(self.estimate)
    }

    fn pixel(&self, row: usize, offset: Pixels) -> Option<f64> {
        let range = self.range?;
        let ix = row
            .checked_sub(range.start)
            .filter(|ix| *ix <= range.rows)?;
        self.prefix
            .get(ix)
            .map(|prefix| prefix + f64::from(f32::from(offset)))
    }

    fn rebuild(&mut self) {
        let Some(range) = self.range else {
            return;
        };
        if self.prefix_dirty || self.prefix.len() != self.heights.len() + 1 {
            self.prefix.clear();
            self.prefix.push(0.);
            let mut height = 0.;
            self.measured_sum = px(0.);
            self.measured_count = 0;
            for row in &self.heights {
                height += f64::from(f32::from(row.unwrap_or(self.estimate)));
                self.prefix.push(height);
                if let Some(row) = row {
                    self.measured_sum += *row;
                    self.measured_count += 1;
                }
            }
            self.prefix_dirty = false;
        }
        let model_max = (self.estimate * range.total as f32
            + self
                .known_correction(self.estimate)
                .unwrap_or(self.tail + self.padding)
            - self.viewport)
            .max(px(0.));
        let scale = if model_max > px(0.) {
            f64::from(f32::from(self.max / model_max))
        } else {
            0.
        };
        let before = f64::from(f32::from(self.estimate)) * range.start as f64;
        let mut left = Knot {
            pixel: 0.,
            track: (before * scale).clamp(0., f64::from(f32::from(self.max))),
        };
        let mut right = Knot {
            pixel: self.pixel(range.start + range.rows, px(0.)).unwrap(),
            track: 0.,
        };
        if range.start + range.rows == range.total {
            right.pixel = (right.pixel
                + f64::from(f32::from(self.tail + self.padding - self.viewport)))
            .max(0.);
        }
        right.track =
            ((before + right.pixel) * scale).clamp(left.track, f64::from(f32::from(self.max)));
        right.track = right.track.max(left.track);
        let pivot = self.anchor.and_then(|anchor| {
            self.pixel(anchor.row, anchor.offset).map(|pixel| Knot {
                pixel,
                track: f64::from(f32::from(anchor.track)),
            })
        });
        if let Some(pivot) = pivot.filter(|p| p.pixel >= 0. && p.pixel <= right.pixel) {
            // Window boundaries are estimates, not actual document edges.
            // Recompute them around the explicit seek: eviction must never
            // exclude it merely because tall rows moved it past an ordinal.
            let before = f64::from(f32::from(self.estimate)) * range.start as f64;
            let after = f64::from(f32::from(self.estimate))
                * range.total.saturating_sub(range.start + range.rows) as f64;
            left.track = if before + pivot.pixel > 0. {
                pivot.track * before / (before + pivot.pixel)
            } else {
                0.
            };
            let remaining = (right.pixel - pivot.pixel).max(0.);
            right.track = if after + remaining > 0. {
                f64::from(f32::from(self.max))
                    - (f64::from(f32::from(self.max)) - pivot.track) * after / (after + remaining)
            } else {
                f64::from(f32::from(self.max))
            };
        }
        self.mapping = Mapping::new(left, right, pivot);
    }

    /// Kit queries during wheel bubbling too, before the chat restores its
    /// painted position. A query must not commit that transient anchor.
    pub fn position(&self, top: ListOffset) -> Pixels {
        let range = self.range.unwrap();
        let row = range.start + top.item_ix;
        let pixel = self
            .pixel(row, top.offset_in_item)
            .unwrap_or(self.mapping.right.pixel);
        px(self.mapping.convert(pixel, false) as f32).clamp(px(0.), self.max)
    }

    /// Only a user-requested seek owns a track anchor. Reusing every painted
    /// wheel position here would accumulate wrong unknown-row estimates and
    /// compress the remaining track into the final few physical pixels.
    pub fn pin(&mut self, top: ListOffset, track: Pixels) {
        let Some(range) = self.range else {
            return;
        };
        self.anchor = Some(Anchor {
            row: range.start + top.item_ix,
            offset: top.offset_in_item,
            track,
        });
        self.rebuild();
    }

    pub fn release_pin(&mut self) {
        if self.anchor.take().is_some() {
            self.rebuild();
        }
    }

    pub fn measured_height(&self, row: usize) -> Option<Pixels> {
        let ix = row.checked_sub(self.range?.start)?;
        if self.stale.get(ix).copied().unwrap_or(false) {
            None
        } else {
            self.heights.get(ix).copied().flatten()
        }
    }

    pub fn invalidate(&mut self, rows: std::ops::Range<usize>) {
        self.stale.resize(self.heights.len(), false);
        for ix in rows {
            if let Some(stale) = self.stale.get_mut(ix) {
                *stale = true;
            }
        }
        self.invalidated = true;
    }

    pub fn target(&self, track: Pixels) -> Option<super::ScrollTarget> {
        let value = f64::from(f32::from(track));
        let range = self.range?;
        let ordinal = if value < self.mapping.left.track {
            Some(value / self.mapping.left.track * range.start as f64)
        } else if value >= self.mapping.right.track && range.start + range.rows < range.total {
            Some(
                (range.start + range.rows) as f64
                    + (value - self.mapping.right.track)
                        / (f64::from(f32::from(self.max)) - self.mapping.right.track)
                        * (range.total - range.start - range.rows) as f64,
            )
        } else {
            None
        };
        if let Some(ordinal) = ordinal {
            return Some(super::ScrollTarget {
                row: ordinal.floor() as usize,
                fraction: ordinal.fract() as f32,
            });
        }
        let mut pixel = self.mapping.convert(value, true).max(0.);
        for ix in 0..range.rows {
            let height = f64::from(f32::from(self.height(ix)));
            if pixel < height {
                return Some(super::ScrollTarget {
                    row: range.start + ix,
                    fraction: (pixel / height) as f32,
                });
            }
            pixel -= height;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_mapping_is_reversible_and_preserves_both_edges_and_anchor() {
        let map = Mapping::new(
            Knot {
                pixel: 0.,
                track: 9000.,
            },
            Knot {
                pixel: 1200.,
                track: 10000.,
            },
            Some(Knot {
                pixel: 300.,
                track: 9400.,
            }),
        );
        for (pixel, track) in [(0., 9000.), (300., 9400.), (1200., 10000.)] {
            assert_eq!(map.convert(pixel, false), track);
        }
        for pixel in (0..1200).step_by(8) {
            let track = map.convert(pixel as f64, false);
            assert!((map.convert(track, true) - pixel as f64).abs() < 1e-8);
        }
    }

    #[test]
    fn zero_length_endpoint_does_not_divide_by_zero() {
        let edge = Knot {
            pixel: 0.,
            track: 0.,
        };
        let map = Mapping::new(edge, edge, Some(edge));
        assert_eq!(map.convert(0., false), 0.);
        assert_eq!(map.convert(0., true), 0.);
    }

    #[test]
    fn heterogeneous_page_eviction_keeps_an_explicit_seek_anchor_and_remote_seeks() {
        let mut geometry = Geometry {
            range: Some(HistoryRange {
                start: 0,
                rows: 200,
                total: 1000,
            }),
            heights: (0..200)
                .map(|ix| Some(px(if ix < 100 { 10. } else { 1000. })))
                .collect(),
            estimate: px(100.),
            max: px(100000.),
            ..Default::default()
        };
        geometry.rebuild();
        let top = ListOffset {
            item_ix: 100,
            offset_in_item: px(5.),
        };
        let before = geometry.position(top);
        geometry.pin(top, before);
        // Affine physical progress, not row 100's ordinal position. The
        // fixture's held extent is 100000, while its current model is 181000.
        assert!((before - px(1005. * 100000. / 181000.)).abs() < px(0.001));
        let list = ListState::new(201, gpui_kit::ListAlignment::Top, px(400.));
        geometry.refresh(
            &list,
            HistoryRange {
                start: 50,
                rows: 200,
                total: 1000,
            },
            px(100.),
            px(100000.),
            px(0.),
            std::iter::empty(),
        );
        let after = geometry.position(ListOffset {
            item_ix: 50,
            offset_in_item: px(5.),
        });
        assert!((after - before).abs() < px(0.001));
        let left = geometry.mapping.left.track;
        let right = geometry.mapping.right.track;
        assert!(left < f64::from(f32::from(after)) && right > f64::from(f32::from(after)));
        assert!(geometry.target(px((left / 2.) as f32)).unwrap().row < 50);
        assert!(
            geometry
                .target(px(((right + 100000.) / 2.) as f32))
                .unwrap()
                .row
                >= 250
        );
        assert_eq!(geometry.heights.len(), 200);
    }

    #[test]
    fn remeasurement_marks_only_invalidated_cached_heights_as_stale() {
        let mut geometry = Geometry {
            range: Some(HistoryRange {
                start: 50,
                rows: 3,
                total: 100,
            }),
            heights: vec![Some(px(36.)), Some(px(960.)), Some(px(180.))],
            ..Default::default()
        };
        geometry.invalidate(1..2);
        assert_eq!(geometry.measured_height(50), Some(px(36.)));
        assert_eq!(geometry.measured_height(51), None);
        assert_eq!(geometry.measured_height(52), Some(px(180.)));
    }
}
