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
    readiness: Vec<crate::ui::layout_readiness::Observation>,
    fraction_ready: Vec<bool>,
}

impl Measurements {
    pub fn set_range(&mut self, range: Option<HistoryRange>) {
        if self.range == range {
            return;
        }
        let old_range = self.range;
        let old_readiness = std::mem::take(&mut self.readiness);
        self.readiness = range.map_or_else(Vec::new, |range| {
            (0..=range.rows)
                .map(|index| {
                    old_range
                        .and_then(|old| {
                            (range.start + index)
                                .checked_sub(old.start)
                                .filter(|old_index| {
                                    *old_index < old.rows
                                        || (*old_index == old.rows && index == range.rows)
                                })
                        })
                        .and_then(|index| old_readiness.get(index))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect()
        });
        self.range = range;
        self.epoch = self.epoch.wrapping_add(1);
        self.pending = vec![None; range.map_or(0, |range| range.rows + 1)];
        self.fraction_ready = vec![false; self.pending.len()];
        self.dirty.clear();
    }

    pub fn invalidate(&mut self, rows: Range<usize>) {
        self.epoch = self.epoch.wrapping_add(1);
        for index in rows {
            if let Some(height) = self.pending.get_mut(index) {
                *height = None;
                self.fraction_ready[index] = false;
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

    pub fn drain(&mut self) -> impl Iterator<Item = (usize, Pixels, bool)> + '_ {
        let Self {
            pending,
            dirty,
            fraction_ready,
            ..
        } = self;
        dirty.drain(..).filter_map(|index| {
            pending[index]
                .take()
                .map(|height| (index, height, fraction_ready[index]))
        })
    }

    pub fn fraction_ready(&self, index: usize) -> bool {
        self.fraction_ready.get(index).copied().unwrap_or(false)
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
    previous_height: Option<Pixels>,
    context_complete: bool,
}

impl MeasuredRow {
    pub fn new(
        child: AnyElement,
        width: Rc<Cell<Pixels>>,
        measurements: Rc<RefCell<Measurements>>,
        range: HistoryRange,
        index: usize,
        previous_height: Option<Pixels>,
        context_complete: bool,
    ) -> Self {
        let epoch = measurements.borrow().epoch;
        Self {
            child,
            width,
            measurements,
            range,
            epoch,
            index,
            previous_height,
            context_complete,
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
    type RequestLayoutState = bool;
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
    ) -> (LayoutId, bool) {
        // As in Collapse, measure before constructing the outer leaf. Nested
        // compute_layout inside a request_measured_layout callback is invalid.
        let previous = self
            .measurements
            .borrow()
            .readiness
            .get(self.index)
            .cloned()
            .unwrap_or_default();
        let (natural, observation) = crate::ui::layout_readiness::observe(&previous, || {
            self.child.layout_as_root(
                size(
                    AvailableSpace::Definite(self.width.get()),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            )
        });
        // A clipped semantic group or footer lacks its surrounding context.
        // Its temporary layout is not a document-level height fact, including
        // when its owner is an interior raw row of the resident window.
        let ready = observation.is_ready() && self.context_complete;
        let current = {
            let measurements = self.measurements.borrow();
            measurements.range == Some(self.range) && measurements.epoch == self.epoch
        };
        if current {
            let mut measurements = self.measurements.borrow_mut();
            measurements.readiness[self.index] = observation;
            measurements.fraction_ready[self.index] = ready;
        }
        // A remounted asynchronous surface may briefly lay out source text or
        // an incomplete nested preview. Keep its compatible scalar placeholder
        // until the rendered layout is ready; it cannot become a fresh fact.
        // A canonical scalar can belong to a hidden member of a clipped tool
        // group, while this partial page temporarily renders its header here.
        // Hold only asynchronous content with the same semantic surroundings.
        let held = current && self.context_complete && !ready && self.previous_height.is_some();
        let height = if held {
            self.previous_height.unwrap()
        } else {
            natural.height
        };
        if current {
            self.measurements
                .borrow_mut()
                .record(self.range, self.epoch, self.index, height);
        }
        let mut style = Style::default();
        style.flex_shrink = 0.;
        style.size.width = natural.width.into();
        style.size.height = height.into();
        (window.request_layout(style, None, cx), held)
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        held: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(held.then_some(ContentMask { bounds }), |window| {
            self.child.prepaint_at(bounds.origin, window, cx);
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        held: &mut bool,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(held.then_some(ContentMask { bounds }), |window| {
            self.child.paint(window, cx);
        });
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
        assert_eq!(
            measurements.drain().collect::<Vec<_>>(),
            vec![(1, px(0.), false)]
        );
        measurements.set_range(Some(HistoryRange { start: 60, ..range }));
        measurements.record(range, epoch, 2, px(999.));
        assert!(measurements.drain().next().is_none());
        assert_eq!(measurements.pending.len(), range.rows + 1);
    }
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use crate::ui::layout_readiness::{Token, track};

    struct Pending(Token);
    impl Render for Pending {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            track(&self.0);
            div().w(px(320.)).h(px(1042.))
        }
    }

    #[::core::prelude::v1::test]
    fn a_partial_window_footer_is_not_published_as_a_changed_document_height() {
        let mut app = ::gpui::TestAppContext::single();
        let cx = app.add_empty_window();
        let range = HistoryRange {
            start: 150,
            rows: 50,
            total: 245,
        };
        let measurements = Rc::new(RefCell::new(Measurements::default()));
        measurements.borrow_mut().set_range(Some(range));
        let paint = |cx: &mut ::gpui::VisualTestContext, complete| {
            cx.draw(point(px(0.), px(0.)), size(px(320.), px(700.)), |_, _| {
                MeasuredRow::new(
                    div().h(px(52.)).into_any_element(),
                    Rc::new(Cell::new(px(320.))),
                    measurements.clone(),
                    range,
                    49,
                    Some(px(82.)),
                    complete,
                )
            })
            .0
        };
        assert!(!paint(cx, false));
        assert_eq!(
            measurements.borrow_mut().drain().collect::<Vec<_>>(),
            vec![(49, px(52.), false)]
        );
        assert!(!paint(cx, true));
        assert_eq!(
            measurements.borrow_mut().drain().collect::<Vec<_>>(),
            vec![(49, px(52.), true)]
        );
    }

    #[::core::prelude::v1::test]
    fn paging_preserves_only_the_readiness_of_overlapping_raw_rows() {
        let mut measurements = Measurements::default();
        measurements.set_range(Some(HistoryRange {
            start: 20,
            rows: 2,
            total: 100,
        }));
        let token = Token::default();
        measurements.readiness[1] =
            crate::ui::layout_readiness::observe(&Default::default(), || track(&token)).1;
        measurements.set_range(Some(HistoryRange {
            start: 21,
            rows: 2,
            total: 100,
        }));
        assert!(!measurements.readiness[0].is_ready());
        assert!(measurements.readiness[1].is_ready());
        measurements.invalidate(0..1);
        assert!(!measurements.readiness[0].is_ready());
        measurements.set_range(Some(HistoryRange {
            start: 80,
            rows: 2,
            total: 100,
        }));
        assert!(measurements.readiness.iter().all(|row| row.is_ready()));
    }

    #[::core::prelude::v1::test]
    fn remounted_and_retained_pending_content_keeps_its_placeholder_without_publishing_a_fact() {
        let mut app = ::gpui::TestAppContext::single();
        let cx = app.add_empty_window();
        let range = HistoryRange {
            start: 20,
            rows: 1,
            total: 21,
        };
        let measurements = Rc::new(RefCell::new(Measurements::default()));
        measurements.borrow_mut().set_range(Some(range));
        let width = Rc::new(Cell::new(px(320.)));
        let token = Token::default();
        let view = cx.update(|_, cx| cx.new(|_| Pending(token.clone())));
        let make = |child| {
            MeasuredRow::new(
                child,
                width.clone(),
                measurements.clone(),
                range,
                0,
                Some(px(437.)),
                true,
            )
        };
        let (held, ()) = cx.draw(point(px(0.), px(0.)), size(px(320.), px(1200.)), |_, _| {
            make(view.into_any_element())
        });
        assert!(held);
        assert_eq!(
            measurements.borrow_mut().drain().collect::<Vec<_>>(),
            vec![(0, px(437.), false)]
        );
        // The retained subtree can skip Render entirely. Its previous owner's
        // token must still prevent this temporary height from being committed.
        let (held, ()) = cx.draw(point(px(0.), px(0.)), size(px(320.), px(1200.)), |_, _| {
            make(div().h(px(809.)).into_any_element())
        });
        assert!(held);
        assert_eq!(
            measurements.borrow_mut().drain().collect::<Vec<_>>(),
            vec![(0, px(437.), false)]
        );
        token.set_ready(true);
        let (held, ()) = cx.draw(point(px(0.), px(0.)), size(px(320.), px(1200.)), |_, _| {
            make(div().h(px(437.)).into_any_element())
        });
        assert!(!held);
        assert_eq!(
            measurements.borrow_mut().drain().collect::<Vec<_>>(),
            vec![(0, px(437.), true)]
        );
    }
}
