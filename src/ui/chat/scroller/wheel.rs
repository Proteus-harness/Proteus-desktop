//! Keep wheel movement inside the pixels measured by the previous layout.
//!
//! Like Zed's virtual list, GPUI gives brand new rows zero height until their
//! first measurement. A wheel burst past the overdraw can therefore skip a
//! whole unmeasured page. Replay the requested pixels between real layouts.
use std::{cell::Cell, rc::Rc};

use gpui_kit::{ListOffset, ListState, Pixels, px};

#[derive(Default)]
pub(super) struct WheelScroll {
    pub(super) painted: Rc<Cell<Option<ListOffset>>>,
    anchor: Option<ListOffset>,
    remaining: Pixels,
}

impl WheelScroll {
    /// Called after the native list's bubble handler, so nested scroll areas
    /// retain first refusal. Undo its estimated movement before replaying it.
    pub(super) fn push(&mut self, delta: Pixels, list: &ListState) {
        if let Some(top) = self.painted.get().or(self.anchor) {
            list.scroll_to(top);
            self.remaining -= delta;
        }
    }

    /// At most one step per layout, shorter than the list's measured overdraw.
    pub(super) fn step(&mut self, list: &ListState, overdraw: Pixels) -> bool {
        if self.remaining != px(0.) && self.painted.take().is_some() {
            let distance = self.remaining.clamp(-overdraw / 2., overdraw / 2.);
            self.remaining -= distance;
            list.scroll_by(distance);
            self.anchor = Some(list.logical_scroll_top());
        }
        self.remaining != px(0.)
    }

    pub(super) fn invalidate(&mut self, list: &ListState) {
        // Keep the splice-adjusted anchor for input arriving before layout,
        // but do not replay against its new, still unmeasured neighbours.
        self.anchor = Some(list.logical_scroll_top());
        self.painted.set(None);
    }

    pub(super) fn clear(&mut self, list: &ListState) {
        self.remaining = px(0.);
        self.invalidate(list);
    }
}

#[cfg(test)]
mod tests {
    use super::WheelScroll;
    use gpui_kit::{ListAlignment, ListOffset, ListState, px};

    #[test]
    fn a_burst_preserves_every_pixel_but_waits_for_layout_between_steps() {
        let list =
            ListState::new(1000, ListAlignment::Top, px(400.)).with_uniform_item_height(px(20.));
        let mut wheel = WheelScroll::default();
        wheel.painted.set(Some(list.logical_scroll_top()));
        for _ in 0..3 {
            // The native handler already moved before the parent's handler.
            list.scroll_by(px(180.));
            wheel.push(px(-180.), &list);
        }
        assert_eq!(list.logical_scroll_top().item_ix, 0);
        assert!(wheel.step(&list, px(400.)));
        assert_eq!(list.logical_scroll_top().item_ix, 10);
        assert!(wheel.step(&list, px(400.)));
        assert_eq!(list.logical_scroll_top().item_ix, 10);
        wheel.painted.set(Some(list.logical_scroll_top()));
        assert!(wheel.step(&list, px(400.)));
        wheel.painted.set(Some(list.logical_scroll_top()));
        assert!(!wheel.step(&list, px(400.)));
        assert_eq!(list.logical_scroll_top().item_ix, 27);
        assert_eq!(list.logical_scroll_top().offset_in_item, px(0.));
    }

    #[test]
    fn opposite_input_cancels_queued_motion_and_a_seek_discards_it() {
        let list =
            ListState::new(100, ListAlignment::Top, px(400.)).with_uniform_item_height(px(20.));
        list.scroll_to(ListOffset {
            item_ix: 20,
            offset_in_item: px(7.),
        });
        let mut wheel = WheelScroll::default();
        wheel.painted.set(Some(list.logical_scroll_top()));
        wheel.push(px(-600.), &list);
        wheel.push(px(600.), &list);
        assert!(!wheel.step(&list, px(400.)));
        assert_eq!(list.logical_scroll_top().offset_in_item, px(7.));
        wheel.push(px(-600.), &list);
        wheel.clear(&list);
        wheel.painted.set(Some(list.logical_scroll_top()));
        assert!(!wheel.step(&list, px(400.)));
        assert_eq!(list.logical_scroll_top().item_ix, 20);
    }

    #[test]
    fn page_insertion_and_remeasurement_wait_for_a_fresh_layout_anchor() {
        let list =
            ListState::new(100, ListAlignment::Top, px(400.)).with_uniform_item_height(px(20.));
        list.scroll_to(ListOffset {
            item_ix: 20,
            offset_in_item: px(7.),
        });
        let mut wheel = WheelScroll::default();
        wheel.painted.set(Some(list.logical_scroll_top()));
        wheel.push(px(-600.), &list);
        // Newly prepended rows really have zero height until their layout.
        // The native list preserves the old row, now at resident index 70.
        list.splice(0..0, 50);
        list.remeasure();
        wheel.invalidate(&list);
        // Native input during the page/layout gap must not consume those
        // zero-height rows either. It joins the same pending pixel request.
        list.scroll_by(px(180.));
        wheel.push(px(-180.), &list);
        assert!(wheel.step(&list, px(400.)));
        assert_eq!(list.logical_scroll_top().item_ix, 70);
        assert_eq!(list.logical_scroll_top().offset_in_item, px(7.));
        wheel.painted.set(Some(list.logical_scroll_top()));
        assert!(wheel.step(&list, px(400.)));
        assert_eq!(list.logical_scroll_top().item_ix, 80);
        assert_eq!(list.logical_scroll_top().offset_in_item, px(7.));
    }
}
