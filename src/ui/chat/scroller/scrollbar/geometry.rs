//! Bounded pixel geometry. Unknown rows retain the initial height estimate;
//! learning a height or splicing a page preserves the last painted anchor.
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
                    && p.track > left.track
                    && p.track < right.track
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
    heights: Vec<Option<Pixels>>,
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
    pub fn refresh(
        &mut self,
        list: &ListState,
        range: HistoryRange,
        estimate: Pixels,
        max: Pixels,
        padding: Pixels,
    ) {
        let viewport = list.viewport_bounds().size.height;
        let mut changed = std::mem::take(&mut self.invalidated)
            || self.range != Some(range)
            || self.estimate != estimate
            || self.viewport != viewport
            || self.max != max
            || self.padding != padding;
        if self.range != Some(range) {
            let old = self.range;
            self.heights = (range.start..range.start + range.rows)
                .map(|row| {
                    old.and_then(|old| row.checked_sub(old.start))
                        .and_then(|ix| self.heights.get(ix))
                        .copied()
                        .flatten()
                })
                .collect();
            self.range = Some(range);
            self.tail = px(0.);
        }
        self.estimate = estimate;
        self.viewport = viewport;
        self.max = max;
        self.padding = padding;
        // bounds_for_item is intentionally unavailable above the viewport.
        // Retain those already measured heights, bounded by the resident page.
        for ix in list.logical_scroll_top().item_ix..range.rows {
            let Some(bounds) = list.bounds_for_item(ix) else {
                break;
            };
            let height = bounds.size.height.max(px(0.));
            if self.heights[ix] != Some(height) {
                self.heights[ix] = Some(height);
                changed = true;
            }
        }
        if let Some(bounds) = list.bounds_for_item(range.rows) {
            changed |= self.tail != bounds.size.height;
            self.tail = bounds.size.height;
        }
        if changed {
            self.rebuild();
        }
    }

    fn height(&self, ix: usize) -> Pixels {
        self.heights[ix].unwrap_or(self.estimate)
    }

    fn pixel(&self, row: usize, offset: Pixels) -> Option<f64> {
        let range = self.range?;
        let ix = row
            .checked_sub(range.start)
            .filter(|ix| *ix <= range.rows)?;
        Some(
            (0..ix)
                .map(|i| f64::from(f32::from(self.height(i))))
                .sum::<f64>()
                + f64::from(f32::from(offset)),
        )
    }

    fn rebuild(&mut self) {
        let Some(range) = self.range else {
            return;
        };
        let mut left = Knot {
            pixel: 0.,
            track: f64::from(f32::from(self.estimate)) * range.start as f64,
        };
        let mut right = Knot {
            pixel: self.pixel(range.start + range.rows, px(0.)).unwrap(),
            track: f64::from(f32::from(self.estimate)) * (range.start + range.rows) as f64,
        };
        if range.start + range.rows == range.total {
            right.pixel = (right.pixel
                + f64::from(f32::from(self.tail + self.padding - self.viewport)))
            .max(0.);
            right.track = f64::from(f32::from(self.max));
        }
        right.track = right.track.max(left.track);
        let pivot = self.anchor.and_then(|anchor| {
            self.pixel(anchor.row, anchor.offset).map(|pixel| Knot {
                pixel,
                track: f64::from(f32::from(anchor.track)),
            })
        });
        if let Some(pivot) = pivot.filter(|p| p.pixel >= 0. && p.pixel <= right.pixel) {
            // Window boundaries are estimates, not actual document edges.
            // Recompute them around the painted anchor: eviction must never
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

    pub fn position(&mut self, top: ListOffset) -> Pixels {
        let range = self.range.unwrap();
        let row = range.start + top.item_ix;
        let pixel = self
            .pixel(row, top.offset_in_item)
            .unwrap_or(self.mapping.right.pixel);
        let track = px(self.mapping.convert(pixel, false) as f32).clamp(px(0.), self.max);
        self.anchor = Some(Anchor {
            row,
            offset: top.offset_in_item,
            track,
        });
        track
    }

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

    pub fn measured_height(&self, row: usize) -> Option<Pixels> {
        let ix = row.checked_sub(self.range?.start)?;
        self.heights.get(ix).copied().flatten()
    }

    pub fn invalidate(&mut self, rows: std::ops::Range<usize>) {
        for ix in rows {
            if let Some(height) = self.heights.get_mut(ix) {
                *height = None;
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
    fn heterogeneous_page_eviction_keeps_the_painted_anchor_and_remote_seeks() {
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
        let before = geometry.position(ListOffset {
            item_ix: 100,
            offset_in_item: px(5.),
        });
        // The row ordinal and its pixel progress differ by orders of magnitude.
        assert!(before < px(300.));
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
    fn remeasurement_drops_only_the_invalidated_cached_heights() {
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
