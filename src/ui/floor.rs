//! Scrolled content that keeps its height while a block in it closes under a
//! scrolled-down viewport. Without it the scroll position runs past the new
//! end, gets clamped and the whole page jumps; with it the freed room stays
//! empty below the content and goes away as the user scrolls back up.

use gpui_kit::*;

/// Content of a vertical scroll area, centred horizontally.
pub(in crate::ui) struct Floor {
    id: ElementId,
    child: AnyElement,
}

#[derive(Clone, Copy, Default)]
struct FloorState {
    floor: Pixels,
}

/// The same retained room for a virtual list, whose rows cannot be wrapped
/// in `Floor`. Render `height()` as an ordinary empty last row. Reserve one
/// viewport before a disclosure changes size, then trim it after layout;
/// no scroll offset needs to be restored or corrected.
#[derive(Default)]
pub(in crate::ui) struct TailFloor {
    height: Pixels,
}

impl TailFloor {
    pub(in crate::ui) fn height(&self) -> Pixels {
        self.height
    }

    /// A disclosure below a visible header can free at most one viewport
    /// that must remain in view. This guard also covers its first frame,
    /// before the virtual list has measured the smaller rows.
    pub(in crate::ui) fn hold(&mut self, viewport: Pixels) -> bool {
        self.replace(self.height.max(viewport))
    }

    /// `visible` is the room between the natural content end and viewport
    /// bottom. Scrolling up or new content consumes the retained blank tail.
    pub(in crate::ui) fn settle(&mut self, visible: Pixels) -> bool {
        self.replace(retained_height(px(0.), visible, self.height))
    }

    pub(in crate::ui) fn clear(&mut self) -> bool {
        self.replace(px(0.))
    }

    fn replace(&mut self, height: Pixels) -> bool {
        let changed = self.height != height;
        self.height = height;
        changed
    }
}

fn retained_height(natural: Pixels, visible: Pixels, previous: Pixels) -> Pixels {
    natural.max(visible.max(px(0.)).min(previous))
}

pub(in crate::ui) fn floor(id: impl Into<ElementId>, child: impl IntoElement) -> Floor {
    Floor {
        id: id.into(),
        child: child.into_any_element(),
    }
}

impl IntoElement for Floor {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Floor {
    type RequestLayoutState = LayoutId;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let floor = window.with_element_state(
            global_id.expect("Floor has an id"),
            |state: Option<FloorState>, _| {
                let state = state.unwrap_or_default();
                (state.floor, state)
            },
        );
        let child = self.child.request_layout(window, cx);
        let mut style = Style::default();
        style.display = Display::Flex;
        style.flex_direction = FlexDirection::Column;
        style.align_items = Some(AlignItems::Center);
        style.size.width = relative(1.).into();
        style.min_size.height = floor.into();
        (window.request_layout(style, [child], cx), child)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let natural = window.layout_bounds(*child).size.height;
        // The part of the content above the viewport's bottom edge must stay,
        // or the scroll position would be clamped; the rest may go.
        let visible = (window.content_mask().bounds.bottom() - bounds.top()).max(px(0.));
        let floor = retained_height(natural, visible, bounds.size.height);
        window.with_element_state(
            global_id.expect("Floor has an id"),
            |_: Option<FloorState>, _| ((), FloorState { floor }),
        );
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{TailFloor, retained_height};
    use gpui_kit::px;

    #[::core::prelude::v1::test]
    fn a_collapsing_page_keeps_only_room_still_visible() {
        assert_eq!(retained_height(px(120.), px(480.), px(900.)), px(480.));
        assert_eq!(retained_height(px(120.), px(240.), px(480.)), px(240.));
        assert_eq!(retained_height(px(120.), px(-20.), px(240.)), px(120.));
    }

    #[::core::prelude::v1::test]
    fn virtual_tail_is_reserved_before_measurement_and_consumed_without_scrolling() {
        let mut tail = TailFloor::default();
        assert!(tail.hold(px(600.)));
        assert_eq!(tail.height(), px(600.));
        assert!(tail.settle(px(340.)));
        assert_eq!(tail.height(), px(340.));
        // A later layout cannot grow a settled reservation. A new toggle
        // explicitly calls hold before its content starts shrinking.
        assert!(!tail.settle(px(400.)));
        assert!(tail.settle(px(160.)));
        assert!(tail.settle(px(-20.)));
        assert_eq!(tail.height(), px(0.));
        assert!(tail.hold(px(600.)));
        assert!(tail.clear());
        assert_eq!(tail.height(), px(0.));
    }
}
