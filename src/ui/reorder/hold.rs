//! A hold belongs to one press, row geometry and authoritative list order.
//! Small pointer jitter is allowed; leaving before the hold arms cancels it.
use gpui_kit::{Bounds, Pixels, Point, SharedString};

pub(super) struct Hold {
    pub origin: Point<Pixels>,
    pub bounds: Bounds<Pixels>,
    source: Vec<SharedString>,
    revision: Option<u64>,
}

impl Hold {
    pub fn new(
        origin: Point<Pixels>,
        bounds: Bounds<Pixels>,
        source: Vec<SharedString>,
        revision: Option<u64>,
    ) -> Self {
        Self {
            origin,
            bounds,
            source,
            revision,
        }
    }

    pub fn contains(&self, position: Point<Pixels>) -> bool {
        self.bounds.contains(&position) && (position - self.origin).magnitude() <= 6.
    }

    pub fn retained(&self, source: &[SharedString], revision: Option<u64>) -> bool {
        self.source == source && self.revision == revision
    }

    /// GPUI's native drag layer anchors to the first dragging mousemove.
    /// Compensate its offset so even a fast first move keeps the grab point.
    pub fn preview_shift(&self, native_offset: Point<Pixels>) -> Point<Pixels> {
        native_offset - (self.origin - self.bounds.origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{point, px, size};

    fn hold() -> Hold {
        Hold::new(
            point(px(110.), px(120.)),
            Bounds::new(point(px(100.), px(100.)), size(px(200.), px(40.))),
            vec!["a".into(), "b".into()],
            Some(3),
        )
    }

    #[::core::prelude::v1::test]
    fn waiting_hold_accepts_jitter_but_not_fast_motion_or_leaving_row() {
        let hold = hold();
        assert!(hold.contains(point(px(110.), px(120.))));
        assert!(hold.contains(point(px(116.), px(120.))));
        assert!(!hold.contains(point(px(116.1), px(120.))));
        assert!(!hold.contains(point(px(250.), px(120.))));
        let near_edge = Hold::new(
            point(px(101.), px(120.)),
            hold.bounds,
            vec!["a".into()],
            None,
        );
        assert!(!near_edge.contains(point(px(99.), px(120.))));
    }

    #[::core::prelude::v1::test]
    fn pending_hold_cannot_transfer_to_a_reordered_removed_or_replaced_row() {
        let hold = hold();
        assert!(hold.retained(&["a".into(), "b".into()], Some(3)));
        assert!(!hold.retained(&["b".into(), "a".into()], Some(3)));
        assert!(!hold.retained(&["b".into()], Some(3)));
        assert!(!hold.retained(&["a".into(), "b".into()], Some(4)));
    }

    #[::core::prelude::v1::test]
    fn preview_preserves_the_original_grab_point_after_a_fast_first_move() {
        let hold = hold();
        let first_move = point(px(260.), px(270.));
        let native_offset = first_move - hold.bounds.origin;
        let ghost_origin = first_move - native_offset + hold.preview_shift(native_offset);
        assert_eq!(
            ghost_origin + (hold.origin - hold.bounds.origin),
            first_move
        );
    }

    fn pressed() -> super::super::State {
        super::super::State {
            pressed: Some("a".into()),
            press_epoch: 7,
            hold: Some(hold()),
            ..Default::default()
        }
    }

    #[::core::prelude::v1::test]
    fn cancellation_cannot_be_rearmed_by_a_pending_timer() {
        let mut state = pressed();
        assert!(state.cancel_motion(true, point(px(250.), px(120.))));
        state.arm(7, point(px(110.), px(120.)), true);
        assert!(!state.armed);
        assert!(state.pressed.is_none());
        assert!(state.hold.is_none());

        for (epoch, active) in [(8, true), (7, false)] {
            let mut state = pressed();
            state.arm(epoch, point(px(110.), px(120.)), active);
            assert!(!state.armed);
            assert!(state.hold.is_none());
        }
    }

    #[::core::prelude::v1::test]
    fn armed_drag_may_leave_source_but_release_outside_always_cancels() {
        let mut state = pressed();
        assert!(!state.cancel_motion(true, point(px(112.), px(120.))));
        state.arm(7, point(px(112.), px(120.)), true);
        assert!(state.armed);
        assert!(!state.cancel_motion(true, point(px(700.), px(800.))));
        assert!(state.armed);
        assert!(state.cancel_motion(false, point(px(700.), px(800.))));
        assert!(!state.armed);
        assert!(state.hold.is_none());
    }

    #[::core::prelude::v1::test]
    fn prepaint_cancellation_blocks_the_previously_rendered_native_listener() {
        let mut state = pressed();
        state.arm(7, point(px(110.), px(120.)), true);
        let installed = state.pressed.clone();
        assert!(!state.stale_drag_listener(installed.as_ref(), 7));
        // Prepaint invalidates the hold after render already installed on_drag.
        state.release();
        assert!(state.stale_drag_listener(installed.as_ref(), 7));
        // The next render has no listener, so ordinary moves can bubble again.
        assert!(!state.stale_drag_listener(None, 7));
    }

    #[::core::prelude::v1::test]
    fn capture_blocks_native_drag_after_release_outside_or_escape_before_render() {
        for release_outside in [true, false] {
            let mut state = pressed();
            state.arm(7, point(px(110.), px(120.)), true);
            let installed = state.pressed.clone();
            let epoch = if release_outside {
                assert!(state.cancel_motion(false, point(px(700.), px(800.))));
                7
            } else {
                // Escape can change the global epoch before the next render.
                8
            };
            assert!(state.stale_drag_listener(installed.as_ref(), epoch));
        }
    }
}
