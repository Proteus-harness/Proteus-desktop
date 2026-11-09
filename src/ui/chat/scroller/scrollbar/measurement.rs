//! Observe the measurement already required by our auto-sized vertical list.
//! GPUI does not expose cached sizes in its leading overdraw through item bounds.
//! Keep those facts separate until the completed list layout commits a batch.
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

use gpui_kit::*;

use super::HistoryRange;

#[derive(Default)]
pub(super) struct Measurements {
    range: Option<HistoryRange>,
    epoch: u64,
    pending: Vec<Option<Pixels>>,
    dirty: Vec<usize>,
}

impl Measurements {
    pub fn set_range(&mut self, range: Option<HistoryRange>) {
        if self.range == range {
            return;
        }
        self.range = range;
        self.epoch = self.epoch.wrapping_add(1);
        self.pending = vec![None; range.map_or(0, |range| range.rows + 1)];
        self.dirty.clear();
    }

    pub fn invalidate(&mut self, rows: Range<usize>) {
        self.epoch = self.epoch.wrapping_add(1);
        for index in rows {
            if let Some(height) = self.pending.get_mut(index) {
                *height = None;
            }
        }
        self.dirty.retain(|index| self.pending[*index].is_some());
    }

    fn record(&mut self, range: HistoryRange, epoch: u64, index: usize, height: Pixels) {
        if self.range != Some(range) || self.epoch != epoch {
            return;
        }
        let Some(slot) = self.pending.get_mut(index) else {
            return;
        };
        if slot.is_none() {
            self.dirty.push(index);
        }
        *slot = Some(height.max(px(0.)));
    }

    pub fn drain(&mut self) -> impl Iterator<Item = (usize, Pixels)> + '_ {
        let Self { pending, dirty, .. } = self;
        dirty
            .drain(..)
            .filter_map(|index| pending[index].take().map(|height| (index, height)))
    }
}

/// Only for a List's Auto vertical sizing: its current parent supplies an exact
/// width before List.prepaint. The child is laid out once, including overdraw;
/// prepaint reuses that layout rather than measuring the child a second time.
pub(super) struct MeasuredRow {
    child: AnyElement,
    width: Rc<Cell<Pixels>>,
    measurements: Rc<RefCell<Measurements>>,
    range: HistoryRange,
    epoch: u64,
    index: usize,
}

impl MeasuredRow {
    pub fn new(
        child: AnyElement,
        width: Rc<Cell<Pixels>>,
        measurements: Rc<RefCell<Measurements>>,
        range: HistoryRange,
        index: usize,
    ) -> Self {
        let epoch = measurements.borrow().epoch;
        Self {
            child,
            width,
            measurements,
            range,
            epoch,
            index,
        }
    }
}

impl IntoElement for MeasuredRow {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for MeasuredRow {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // As in Collapse, measure before constructing the outer leaf. Nested
        // compute_layout inside a request_measured_layout callback is invalid.
        let natural = self.child.layout_as_root(
            size(
                AvailableSpace::Definite(self.width.get()),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        self.measurements
            .borrow_mut()
            .record(self.range, self.epoch, self.index, natural.height);
        let mut style = Style::default();
        style.flex_shrink = 0.;
        style.size.width = natural.width.into();
        style.size.height = natural.height.into();
        (window.request_layout(style, None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint_at(bounds.origin, window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn measurements_are_deduplicated_and_stale_layouts_cannot_publish() {
        let range = HistoryRange {
            start: 50,
            rows: 3,
            total: 100,
        };
        let mut measurements = Measurements::default();
        measurements.set_range(Some(range));
        let epoch = measurements.epoch;
        measurements.record(range, epoch, 0, px(20.));
        measurements.record(range, epoch, 0, px(24.));
        measurements.record(range, epoch, 1, px(0.));
        measurements.invalidate(0..1);
        measurements.record(range, epoch, 0, px(999.));
        assert_eq!(measurements.drain().collect::<Vec<_>>(), vec![(1, px(0.))]);
        measurements.set_range(Some(HistoryRange { start: 60, ..range }));
        measurements.record(range, epoch, 2, px(999.));
        assert!(measurements.drain().next().is_none());
        assert_eq!(measurements.pending.len(), range.rows + 1);
    }
}
