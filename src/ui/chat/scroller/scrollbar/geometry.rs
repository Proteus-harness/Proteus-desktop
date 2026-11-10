//! Native pixel geometry over a sparse, history-wide map of scalar heights.
//! Paging drops widgets, not measurements; only explicit seeks pin the track.
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
    /// Native dimensions are authoritative only in this resident layout.
    /// The global map supplies estimates, never a stale fractional seek.
    heights: Vec<Option<Pixels>>,
    stale: Vec<bool>,
    cache: super::height_cache::HeightCache,
    cache_revision: u64,
    facts_revision: u64,
    prefix: Vec<f64>,
    prefix_dirty: bool,
    resident_delta: f64,
    document_pixels: f64,
    before: f64,
    after: f64,
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
    pub fn exclude_disk(&self, rows: std::ops::Range<usize>) {
        if let Some(range) = self.range {
            let start = range.start + rows.start.min(range.rows);
            let end = range.start + rows.end.min(range.rows);
            self.exclude_document_disk(start..end, range.total);
        }
    }

    pub fn exclude_document_disk(&self, rows: std::ops::Range<usize>, total: usize) {
        self.cache.mutate(rows.clone(), 4, |map, _| {
            map.resize(total);
            map.exclude_disk(rows);
        });
    }

    pub fn unbind_height_cache(&self) {
        self.cache.unbind();
    }

    pub fn bind_height_cache(
        &self,
        key: super::HeightCacheKey,
    ) -> Option<async_channel::Receiver<Option<super::height_store::Snapshot>>> {
        self.cache.bind(key)
    }

    pub fn import_height_cache(
        &mut self,
        snapshot: super::height_store::Snapshot,
        total: usize,
    ) -> bool {
        let protected = self
            .range
            .map_or(0..0, |range| range.start..range.start + range.rows);
        if !self.cache.import(snapshot, total, protected) {
            return false;
        }
        self.prefix_dirty = true;
        self.invalidated = true;
        true
    }

    #[cfg(debug_assertions)]
    pub fn diagnostic_mapping(&self) -> [[f64; 2]; 3] {
        [self.mapping.left, self.mapping.pivot, self.mapping.right].map(|k| [k.pixel, k.track])
    }

    #[cfg(any(debug_assertions, test))]
    pub fn facts_revision(&self) -> u64 {
        self.facts_revision
    }

    #[cfg(debug_assertions)]
    pub fn diagnostic_cache(&self) -> (usize, usize, u64) {
        self.cache
            .read(|map| (map.measured_rows(), map.span_count(), map.revision()))
    }

    pub fn represents(&self, range: HistoryRange) -> bool {
        self.range == Some(range)
    }

    pub fn footer_height(&self) -> Pixels {
        self.tail
    }

    pub fn set_footer_height(&mut self, height: Pixels) {
        if self.tail != height {
            self.tail = height;
            self.invalidated = true;
        }
    }

    pub fn document_height(&self, range: HistoryRange, prior: Pixels, padding: Pixels) -> Pixels {
        let pixels = self.cache.read(|map| {
            map.prefix(range.total)
                + range.total.saturating_sub(map.total_rows()) as f64 * f64::from(f32::from(prior))
        });
        let local = if self.range == Some(range) {
            self.resident_delta
        } else {
            0.
        };
        px((pixels + local) as f32) + self.tail + padding
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
        let viewport = list.viewport_bounds().size.height;
        let mut changed = std::mem::take(&mut self.invalidated)
            || self.range != Some(range)
            || self.estimate != estimate
            || self.viewport != viewport
            || self.max != max
            || self.padding != padding;
        if self.range != Some(range) {
            let old = self.range;
            let cache = self.cache.clone();
            let heights = cache.read(|map| {
                (range.start..range.start + range.rows)
                    .map(|row| {
                        old.and_then(|old| row.checked_sub(old.start))
                            .and_then(|ix| self.heights.get(ix))
                            .copied()
                            .flatten()
                            .or_else(|| map.lookup(row).map(|(height, _)| px(height as f32)))
                    })
                    .collect()
            });
            // A remounted cached row still waits for native layout before a
            // fractional seek can use it. Scalars do not keep widgets alive.
            self.stale = vec![true; range.rows];
            self.heights = heights;
            self.range = Some(range);
            self.prefix_dirty = true;
        }
        self.estimate = estimate;
        self.viewport = viewport;
        self.max = max;
        self.padding = padding;
        let mut updates = Vec::new();
        let cache = self.cache.clone();
        cache.read(|map| {
            for (ix, height) in measurements {
                if ix == range.rows {
                    changed |= self.tail != height;
                    self.tail = height;
                } else if let Some(cached) = self.heights.get_mut(ix) {
                    if *cached != Some(height) {
                        #[cfg(debug_assertions)]
                        super::scroll_trace::record(|| {
                            serde_json::json!({
                            "phase":"height", "row":range.start+ix,
                            "before":cached.map(f32::from), "after":f32::from(height)})
                        });
                        *cached = Some(height);
                        self.prefix_dirty = true;
                        changed = true;
                    }
                    changed |= std::mem::replace(&mut self.stale[ix], false);
                    let value = f64::from(f32::from(height));
                    if map.lookup(range.start + ix) != Some((value, true)) {
                        updates.push((range.start + ix, value));
                    }
                }
            }
        });
        let resize = cache.read(|map| map.total_rows() != range.total);
        if resize || !updates.is_empty() {
            let protected = range.start..range.start + range.rows;
            cache.mutate(protected.clone(), updates.len() * 2 + 2, |map, allowed| {
                map.resize(range.total);
                for (row, height) in updates {
                    map.set_limited(row, height, allowed, protected.clone());
                }
            });
        }
        if self.cache_revision != cache.revision() {
            changed = true;
            self.prefix_dirty = true;
            self.facts_revision = self.facts_revision.wrapping_add(1);
        }
        if changed {
            self.rebuild();
        }
        changed
    }

    fn height(&self, ix: usize) -> Pixels {
        self.heights[ix].unwrap_or_else(|| {
            let row = self.range.unwrap().start + ix;
            px(self.cache.read(|map| {
                map.estimate(row)
                    .unwrap_or(f64::from(f32::from(self.estimate)))
            }) as f32)
        })
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
        if self.prefix_dirty
            || self.cache_revision != self.cache.revision()
            || self.prefix.len() != self.heights.len() + 1
        {
            let cache = self.cache.clone();
            cache.read(|map| {
                self.prefix.clear();
                self.prefix.push(0.);
                let mut height = 0.;
                self.resident_delta = 0.;
                for (ix, value) in self.heights.iter().enumerate() {
                    let estimate = map
                        .estimate(range.start + ix)
                        .unwrap_or(f64::from(f32::from(self.estimate)));
                    let value = value
                        .map(|height| f64::from(f32::from(height)))
                        .unwrap_or(estimate);
                    height += value;
                    self.resident_delta += value - estimate;
                    self.prefix.push(height);
                }
                self.before = map.prefix(range.start);
                self.after = map.total_height() - map.prefix(range.start + range.rows);
                self.document_pixels = map.total_height() + self.resident_delta;
                self.cache_revision = map.revision();
            });
            self.prefix_dirty = false;
        }
        let model_max = (self.document_pixels
            + f64::from(f32::from(self.tail + self.padding - self.viewport)))
        .max(0.);
        let max = f64::from(f32::from(self.max));
        let scale = if model_max > 0. { max / model_max } else { 0. };
        let mut left = Knot {
            pixel: 0.,
            track: (self.before * scale).clamp(0., max),
        };
        let mut right = Knot {
            pixel: self.prefix.last().copied().unwrap_or(0.),
            track: 0.,
        };
        if range.start + range.rows == range.total {
            right.pixel = (right.pixel
                + f64::from(f32::from(self.tail + self.padding - self.viewport)))
            .max(0.);
        }
        right.track = ((self.before + right.pixel) * scale).clamp(left.track, max);
        let pivot = self.anchor.and_then(|anchor| {
            self.pixel(anchor.row, anchor.offset).map(|pixel| Knot {
                pixel,
                track: f64::from(f32::from(anchor.track)),
            })
        });
        if let Some(pivot) = pivot.filter(|p| p.pixel >= 0. && p.pixel <= right.pixel) {
            left.track = if self.before + pivot.pixel > 0. {
                pivot.track * self.before / (self.before + pivot.pixel)
            } else {
                0.
            };
            let remaining = (right.pixel - pivot.pixel).max(0.);
            right.track = if self.after + remaining > 0. {
                max - (max - pivot.track) * self.after / (self.after + remaining)
            } else {
                max
            };
        }
        self.mapping = Mapping::new(left, right, pivot);
    }

    pub fn position(&self, top: ListOffset) -> Pixels {
        let range = self.range.unwrap();
        let pixel = self
            .pixel(range.start + top.item_ix, top.offset_in_item)
            .unwrap_or(self.mapping.right.pixel);
        px(self.mapping.convert(pixel, false) as f32).clamp(px(0.), self.max)
    }

    /// Queries between paging and layout use the same global cache, without
    /// committing the native wheel observer's transient position.
    pub fn estimated_position(&self, row: usize, offset: Pixels) -> Pixels {
        px(self.cache.read(|map| {
            map.prefix(row)
                + row.saturating_sub(map.total_rows()) as f64
                    * f64::from(f32::from(super::UNKNOWN_ROW_HEIGHT))
        }) as f32)
            + offset
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

    pub fn release_pin(&mut self) {
        if self.anchor.take().is_some() {
            self.rebuild();
        }
    }

    pub fn measured_height(&self, row: usize) -> Option<Pixels> {
        let ix = row.checked_sub(self.range?.start)?;
        if self.stale.get(ix).copied().unwrap_or(true) {
            None
        } else {
            self.heights.get(ix).copied().flatten()
        }
    }

    /// Compatible scalar for a remount placeholder, never native seek authority.
    pub fn estimated_height(&self, row: usize) -> Option<Pixels> {
        self.cache
            .read(|map| map.lookup(row).map(|(height, _)| px(height as f32)))
    }

    pub fn invalidate(&mut self, rows: std::ops::Range<usize>) {
        if let Some(range) = self.range {
            let end = rows.end.min(range.rows);
            let start = rows.start.min(end);
            self.invalidate_document_rows(range.start + start..range.start + end);
        }
    }

    pub fn invalidate_document_rows(&mut self, rows: std::ops::Range<usize>) {
        if let Some(range) = self.range {
            self.stale.resize(self.heights.len(), false);
            for row in rows.start.max(range.start)..rows.end.min(range.start + range.rows) {
                self.stale[row - range.start] = true;
            }
            let protected = range.start..range.start + range.rows;
            self.cache
                .mutate(protected, 2, |map, _| map.invalidate(rows));
        }
        self.invalidated = true;
    }

    pub fn rebase(&mut self, keep_prefix: Option<usize>) {
        let total = self.cache.read(|map| map.total_rows());
        self.cache.mutate(0..0, 2, |map, _| {
            map.rebase(keep_prefix.unwrap_or(0), total)
        });
        self.range = None;
        self.heights.clear();
        self.stale.clear();
        self.resident_delta = 0.;
        self.prefix_dirty = true;
        self.anchor = None;
        self.invalidated = true;
    }

    pub fn reset_layout(&mut self) {
        let total = self.cache.read(|map| map.total_rows());
        self.cache.mutate(0..0, 0, |map, _| {
            map.reset(total, f64::from(f32::from(super::UNKNOWN_ROW_HEIGHT)))
        });
        // Even an equivalent explicit reset may have discarded all scalars;
        // allow the following bind to reload its compatible disk estimates.
        self.cache.unbind();
        self.heights.fill(None);
        self.stale.fill(true);
        self.resident_delta = 0.;
        self.prefix_dirty = true;
        self.anchor = None;
        self.invalidated = true;
    }

    pub fn target(&self, track: Pixels) -> Option<super::ScrollTarget> {
        let value = f64::from(f32::from(track));
        let range = self.range?;
        let global = if value < self.mapping.left.track {
            Some(value / self.mapping.left.track * self.before)
        } else if value >= self.mapping.right.track && range.start + range.rows < range.total {
            let room = f64::from(f32::from(self.max)) - self.mapping.right.track;
            (room > 0.).then(|| {
                self.cache.read(|map| map.prefix(range.start + range.rows))
                    + (value - self.mapping.right.track) / room * self.after
            })
        } else {
            None
        };
        if let Some(global) = global {
            return self
                .cache
                .read(|map| map.target(global))
                .map(|(row, fraction)| super::ScrollTarget {
                    row,
                    fraction: fraction as f32,
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
        let cache = super::super::height_cache::HeightCache::new(1000, 100.);
        cache.mutate(0..200, 400, |map, allowed| {
            for row in 0..200 {
                map.set_limited(row, if row < 100 { 10. } else { 1000. }, allowed, 0..200);
            }
        });
        let mut geometry = Geometry {
            cache,
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
