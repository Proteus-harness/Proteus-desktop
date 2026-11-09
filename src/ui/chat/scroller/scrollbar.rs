//! A scrollbar over a whole paged history, independent of its resident window.
//! Uses the same handle boundary as Zed's crates/ui/src/components/scrollbar.rs;
//! the standard Kit scrollbar still owns painting, hit testing and dragging.
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gpui_kit::{component::scroll::ScrollbarHandle, *};

#[path = "scrollbar/geometry.rs"]
mod geometry;
use geometry::Geometry;
#[path = "scrollbar/measurement.rs"]
mod measurement;
use measurement::{MeasuredRow, Measurements};
#[cfg(debug_assertions)]
#[path = "scroll_trace.rs"]
pub(super) mod scroll_trace;

/// Prior for an unknown ordinary row, used only when the viewport is entirely
/// censored and no ordinary resident measurement exists yet. A 50,000px reply
/// is evidence for that one reply, never the mean of all unknown messages.
const UNKNOWN_ROW_HEIGHT: Pixels = px(100.);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HistoryRange {
    pub start: usize,
    pub rows: usize,
    pub total: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollTarget {
    /// `total` addresses the bottom, rather than the top of the last message.
    pub row: usize,
    pub fraction: f32,
}

#[derive(Clone, Copy, Default)]
struct State {
    range: Option<HistoryRange>,
    unit: Option<Pixels>,
    known_correction: Option<Pixels>,
    pending: Option<ScrollTarget>,
    displayed: Option<(ScrollTarget, Pixels)>,
    local: Option<ScrollTarget>,
    waiting: bool,
    drag_height: Option<Pixels>,
    seek_height: Option<Pixels>,
}

#[derive(Clone)]
pub(crate) struct HistoryScrollbar {
    list: ListState,
    state: Rc<Cell<State>>,
    geometry: Rc<RefCell<Geometry>>,
    measurements: Rc<RefCell<Measurements>>,
    width: Rc<Cell<Pixels>>,
    padding: Pixels,
}

impl HistoryScrollbar {
    pub fn new(list: ListState, padding: Pixels) -> Self {
        Self {
            list,
            state: Rc::default(),
            geometry: Rc::default(),
            measurements: Rc::default(),
            width: Rc::default(),
            padding,
        }
    }

    pub fn set_range(&self, range: Option<HistoryRange>, seeking: bool) -> bool {
        let mut state = self.state.get();
        let changed = state.range != range;
        if state
            .range
            .zip(range)
            .is_some_and(|(old, new)| new.total < old.total)
        {
            // A replacement/rewritten snapshot can remove rows. Its ordinal
            // space no longer owns the old correction or seek anchors; the
            // native splice retains the surviving physical reading position.
            if state.drag_height.is_some() {
                self.list.scrollbar_drag_ended();
            }
            state = State::default();
            *self.geometry.borrow_mut() = Geometry::default();
        }
        if changed {
            self.measurements.borrow_mut().set_range(range);
        }
        state.range = range;
        if range.is_none() {
            state = State::default();
            *self.geometry.borrow_mut() = Geometry::default();
        } else if state.waiting && !seeking {
            state.waiting = false;
            // An accepted page still needs layout and fractional positioning.
            // Keep the requested track anchor until settle pins its real pixels.
            // A cancelled/failed seek has no matching resident target.
            if !state.displayed.is_some_and(|(target, _)| {
                range.is_some_and(|range| {
                    (range.start..range.start + range.rows).contains(&target.row)
                })
            }) {
                state.displayed = None;
                state.seek_height = None;
                self.geometry.borrow_mut().release_pin();
            }
        }
        self.state.set(state);
        changed
    }

    pub fn take_target(&self) -> Option<ScrollTarget> {
        let mut state = self.state.get();
        let target = state.pending.take();
        self.state.set(state);
        target
    }

    pub fn cancel_target(&self) -> bool {
        let mut state = self.state.get();
        let pending = state.waiting || state.pending.is_some() || state.local.is_some();
        state.pending = None;
        state.displayed = None;
        state.local = None;
        state.waiting = false;
        self.state.set(state);
        self.begin_browsing();
        pending
    }

    pub fn invalidate(&self, rows: std::ops::Range<usize>) {
        self.geometry.borrow_mut().invalidate(rows.clone());
        self.measurements.borrow_mut().invalidate(rows);
    }

    /// Called by the viewport before its Auto-sized list prepaints. Reading
    /// ListState here from a row renderer would reborrow GPUI's mutable state.
    pub fn set_viewport_width(&self, width: Pixels) {
        if self.width.replace(width) != width
            && let Some(range) = self.state.get().range
        {
            self.invalidate(0..range.rows + 1);
        }
    }

    /// Observe the row's existing layout, also when GPUI only measures it in
    /// the leading overdraw. Rendering and child measurement still happen once.
    pub fn measure_row(&self, index: usize, child: impl IntoElement) -> AnyElement {
        let child = child.into_any_element();
        let Some(range) = self.state.get().range else {
            return child;
        };
        MeasuredRow::new(
            child,
            self.width.clone(),
            self.measurements.clone(),
            range,
            index,
        )
        .into_any_element()
    }

    /// A vertical reading gesture takes over from an explicit seek. Height
    /// facts may refine the scrollbar estimate; the content anchor stays native.
    pub(super) fn begin_browsing(&self) {
        let mut state = self.state.get();
        let released = state.seek_height.take().is_some();
        if released {
            // Explicit seeks can hold an old display extent. Recalibrate once
            // when ordinary reading takes over, using current committed facts.
            state.unit = None;
            self.state.set(state);
            let unit = self.unit();
            let mut state = self.state.get();
            state.known_correction = self.geometry.borrow().known_correction(unit);
            self.state.set(state);
        }
        self.geometry.borrow_mut().release_pin();
        if released && let Some(range) = self.state.get().range {
            self.refresh_geometry(range);
        }
    }

    /// Scroll resident data immediately; an unloaded target goes to the owner.
    pub fn scroll_to(&self, target: ScrollTarget) -> bool {
        let Some(range) = self.state.get().range else {
            return false;
        };
        let mut state = self.state.get();
        if target.row < range.start || target.row >= range.start + range.rows {
            state.waiting = true;
            state.local = None;
            self.state.set(state);
            return false;
        }
        state.waiting = false;
        state.local = Some(target);
        self.state.set(state);
        self.refresh_geometry(range);
        // Resident drag positions already have real pixel geometry. Do not
        // publish a frame at row offset zero before applying its fraction.
        if target.fraction == 0.
            || self.geometry.borrow().measured_height(target.row).is_some()
            || self
                .list
                .bounds_for_item(target.row - range.start)
                .is_some()
        {
            self.settle();
            return true;
        }
        self.list.scroll_to(ListOffset {
            item_ix: target.row - range.start,
            offset_in_item: px(0.),
        });
        true
    }

    /// A newly fetched row is measured by the next list layout. Apply the
    /// fractional position then, without guessing the actual message height.
    pub fn settle(&self) -> bool {
        let mut state = self.state.get();
        let (Some(target), Some(range)) = (state.local, state.range) else {
            return false;
        };
        let Some(index) = target
            .row
            .checked_sub(range.start)
            .filter(|i| *i < range.rows)
        else {
            state.local = None;
            self.state.set(state);
            return false;
        };
        let height = if target.fraction == 0. {
            px(0.)
        } else if let Some(height) =
            self.geometry
                .borrow()
                .measured_height(target.row)
                .or_else(|| {
                    self.list
                        .bounds_for_item(index)
                        .map(|bounds| bounds.size.height)
                })
        {
            height
        } else {
            return true;
        };
        self.list.scroll_to(ListOffset {
            item_ix: index,
            offset_in_item: height * target.fraction,
        });
        self.refresh_geometry(range);
        if let Some((requested, position)) = state.displayed
            && requested == target
        {
            self.geometry
                .borrow_mut()
                .pin(self.list.logical_scroll_top(), position);
        }
        state.local = None;
        state.unit = self.state.get().unit;
        if state.drag_height.is_none() {
            state.displayed = None;
        }
        self.state.set(state);
        true
    }

    /// Recomputed only after committed layout batches. Include folded rows in
    /// the resident sample: ignoring their zero heights biases raw history high.
    fn unit(&self) -> Pixels {
        if let Some(unit) = self.state.get().unit {
            return unit;
        }
        let viewport = self.list.viewport_bounds();
        let start = self.list.logical_scroll_top().item_ix;
        let rows = self.state.get().range.map_or(0, |range| range.rows);
        if let Some(unit) = self.geometry.borrow().ordinary_height(viewport.size.height) {
            let unit = unit.max(px(1.));
            let mut state = self.state.get();
            state.unit = Some(unit);
            self.state.set(state);
            return unit;
        }
        let mut sample_count = 0;
        let mut sample_height = px(0.);
        for index in start..rows {
            let Some(bounds) = self.list.bounds_for_item(index) else {
                break;
            };
            if bounds.top() >= viewport.bottom() {
                break;
            }
            if bounds.size.height >= px(0.) {
                // A tall clipped reply is much more likely to intersect the
                // viewport than a short row. Extrapolating its full height to
                // every unknown row leaves almost the entire track to cover
                // at the beginning of the history. Omit rows hiding more
                // than a viewport of content. Ordinary clipped replies still
                // contribute, even in the smaller first window layout, so
                // the sample is not biased toward the shortest messages.
                // A censored-only viewport uses ordinary cached measurements
                // or the explicit prior below, not the full clipped reply.
                let visible = (bounds.bottom().min(viewport.bottom())
                    - bounds.top().max(viewport.top()))
                .max(px(0.));
                if bounds.size.height - visible <= viewport.size.height {
                    sample_height += bounds.size.height;
                    sample_count += 1;
                }
            }
        }
        let unit = if sample_count > 0 && sample_height > px(0.) {
            sample_height / sample_count as f32
        } else {
            UNKNOWN_ROW_HEIGHT
        }
        .max(px(1.));
        if viewport.size.height > px(0.) {
            let mut state = self.state.get();
            state.unit = Some(unit);
            self.state.set(state);
        }
        unit
    }

    fn height(&self) -> Pixels {
        let state = self.state.get();
        state.drag_height.or(state.seek_height).unwrap_or_else(|| {
            let base = self.unit() * state.range.map_or(0, |r| r.total) as f32;
            let height = state.known_correction.map_or_else(
                // The first layout has not committed its measurements yet.
                // Keep even a single long reply scrollable during that frame.
                || base.max(self.list.content_size().height + self.padding),
                |correction| base + correction,
            );
            height.max(self.list.viewport_bounds().size.height)
        })
    }

    fn max_offset(&self) -> Pixels {
        (self.height() - self.list.viewport_bounds().size.height).max(px(0.))
    }

    fn refresh_geometry(&self, range: HistoryRange) {
        let unit = self.unit();
        let max = self.max_offset();
        self.geometry.borrow_mut().refresh(
            &self.list,
            range,
            unit,
            max,
            self.padding,
            std::iter::empty(),
        );
    }

    fn at_bottom(&self, range: HistoryRange) -> bool {
        let viewport = self.list.viewport_bounds();
        range.start + range.rows == range.total
            && viewport.size.height > px(0.)
            && self
                .list
                .bounds_for_item(range.rows)
                .is_some_and(|bounds| bounds.bottom() + self.padding <= viewport.bottom())
    }

    /// Offset reads during native wheel bubbling are provisional. Only the
    /// real list layout, alongside the wheel's painted anchor, commits ours.
    pub(super) fn did_layout(&self) {
        let state = self.state.get();
        let Some(range) = state.range else {
            return;
        };
        // The row observer writes into a separate collector during list
        // measurement. Commit it only now; offset queries must never see new
        // heights combined with the mapping of a partially completed layout.
        let unit = self.unit();
        let max = self.max_offset();
        let changed = self.geometry.borrow_mut().refresh(
            &self.list,
            range,
            unit,
            max,
            self.padding,
            self.measurements.borrow_mut().drain(),
        );
        if changed && state.drag_height.is_none() && state.seek_height.is_none() {
            let mut state = self.state.get();
            state.unit = None;
            self.state.set(state);
        }
        let unit = self.unit();
        let mut calibrated = self.state.get();
        calibrated.known_correction = self.geometry.borrow().known_correction(unit);
        self.state.set(calibrated);
        self.refresh_geometry(range);
        // Remote/fractional seeks pin their requested position in settle.
        if state.displayed.is_some() || state.waiting || state.local.is_some() {
            return;
        }
        #[cfg(debug_assertions)]
        scroll_trace::record(|| {
            let top = self.list.logical_scroll_top();
            let geometry = self.geometry.borrow();
            serde_json::json!({"phase":"layout", "range":[range.start,range.rows,range.total],
                "row":range.start+top.item_ix,"row_offset":f32::from(top.offset_in_item),
                "native_y":f32::from(self.list.scroll_px_offset_for_scrollbar().y),
                "unit":self.state.get().unit.map(f32::from),"max":f32::from(self.max_offset()),
                "track":f32::from(geometry.position(top)),"map":geometry.diagnostic_mapping(),
                "facts_revision":geometry.facts_revision()})
        });
    }
}

impl ScrollbarHandle for HistoryScrollbar {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.list.viewport_bounds()
    }

    fn content_size(&self) -> Size<Pixels> {
        if self.state.get().range.is_none() {
            return self.list.content_size();
        }
        size(self.list.viewport_bounds().size.width, self.height())
    }

    fn offset(&self) -> Point<Pixels> {
        let state = self.state.get();
        let Some(range) = state.range else {
            return self.list.offset();
        };
        let max = self.max_offset();
        if let Some((_, offset)) = state.displayed {
            return point(px(0.), -offset.min(max));
        }
        let top = self.list.logical_scroll_top();
        if self.at_bottom(range) {
            return point(px(0.), -max);
        }
        let geometry = self.geometry.borrow();
        let offset = if geometry.represents(range) {
            geometry.position(top)
        } else {
            // A range can change before the list lays out. Until did_layout
            // measures it, this is only an estimate, never a new anchor.
            (self.unit() * (range.start + top.item_ix) as f32 + top.offset_in_item)
                .clamp(px(0.), max)
        };
        point(px(0.), -offset)
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        let Some(range) = self.state.get().range else {
            return self.list.set_offset(offset);
        };
        let held_height = self.height();
        let max = (held_height - self.list.viewport_bounds().size.height).max(px(0.));
        let offset = (-offset.y).clamp(px(0.), max);
        self.refresh_geometry(range);
        let ordinal = f32::from(offset / self.unit());
        let target = if offset >= max {
            ScrollTarget {
                row: range.total,
                fraction: 0.,
            }
        } else if offset <= px(0.) {
            ScrollTarget {
                row: 0,
                fraction: 0.,
            }
        } else if let Some(target) = self.geometry.borrow().target(offset) {
            target
        } else {
            ScrollTarget {
                row: ordinal.floor() as usize,
                fraction: ordinal.fract(),
            }
        };
        let mut state = self.state.get();
        state.seek_height = Some(held_height);
        state.pending = Some(target);
        state.displayed = Some((target, offset));
        self.state.set(state);
    }

    fn start_drag(&self) {
        // A fresh gesture starts from the currently displayed extent/position,
        // including an idle explicit seek whose real rows have since resized.
        let height = self.height();
        let position = -self.offset().y;
        self.begin_browsing();
        let mut state = self.state.get();
        state.drag_height = Some(height);
        state.seek_height = Some(height);
        self.state.set(state);
        if let Some(range) = state.range {
            self.refresh_geometry(range);
            self.geometry
                .borrow_mut()
                .pin(self.list.logical_scroll_top(), position);
        }
        self.list.scrollbar_drag_started();
    }

    fn end_drag(&self) {
        let mut state = self.state.get();
        state.drag_height = None;
        // Keep a seek's display extent while idle: releasing the pointer must
        // not move its visual fraction or the content. New browsing frees it.
        if !state.waiting && state.local.is_none() && state.pending.is_none() {
            state.displayed = None;
        }
        self.state.set(state);
        self.list.scrollbar_drag_ended();
    }
}

#[cfg(test)]
#[path = "scrollbar/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "scrollbar/layout_tests.rs"]
mod layout_tests;

#[cfg(test)]
#[path = "scrollbar/calibration_tests.rs"]
mod calibration_tests;

#[cfg(test)]
#[path = "scrollbar/density_tests.rs"]
mod density_tests;
