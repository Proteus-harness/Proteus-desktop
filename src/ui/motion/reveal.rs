//! The element under every disclosure. A fractional grid row keeps the child
//! in the parent's current layout while its visible height opens or closes.
//! Nested disclosures, wrapping and width changes therefore share one layout
//! generation instead of following heights cached from the previous frame.

use gpui_kit::*;

pub(super) struct Reveal {
    id: ElementId,
    /// `None` while the disclosure rests open.
    progress: Option<f32>,
    preview: Option<AnyElement>,
    expanded: bool,
    popup_origin: Option<PopupOrigin>,
    child: AnyElement,
}

/// The edge attached to a popup's control. Anchor chooses the actual side
/// after positioning/flipping, so an upward popup uncovers from its bottom.
#[derive(Clone, Copy)]
pub(in crate::ui) enum PopupOrigin {
    Bottom,
    Horizontal(Bounds<Pixels>),
    Anchor(Bounds<Pixels>),
}

pub(super) enum Flow {
    Content,
    Preview,
    Clipped { content: LayoutId, visible: bool },
    Popup,
}

impl Reveal {
    pub(super) fn new(id: impl Into<ElementId>, progress: Option<f32>, child: AnyElement) -> Self {
        Self {
            id: id.into(),
            progress: progress.map(|progress| progress.clamp(0., 1.)),
            preview: None,
            expanded: true,
            popup_origin: None,
            child,
        }
    }

    pub(super) fn preview(
        id: impl Into<ElementId>,
        progress: Option<f32>,
        expanded: bool,
        preview: AnyElement,
        child: AnyElement,
    ) -> Self {
        Self {
            id: id.into(),
            progress: progress.map(|progress| progress.clamp(0., 1.)),
            preview: Some(preview),
            expanded,
            popup_origin: None,
            child,
        }
    }

    pub(super) fn popup(id: impl Into<ElementId>, origin: PopupOrigin, child: AnyElement) -> Self {
        Self {
            id: id.into(),
            progress: Some(0.),
            preview: None,
            expanded: true,
            popup_origin: Some(origin),
            child,
        }
    }

    pub(super) fn with_progress(mut self, progress: f32) -> Self {
        self.progress = (progress < 1.).then_some(progress.clamp(0., 1.));
        self
    }

    fn popup_mask(&self, mut bounds: Bounds<Pixels>) -> ContentMask<Pixels> {
        let progress = self.progress.unwrap_or(1.);
        if let Some(PopupOrigin::Horizontal(anchor)) = self.popup_origin {
            let width = bounds.size.width * progress;
            // A popup on the panel's left opens from its right edge; one on
            // the right opens from its left edge. This follows a flipped
            // resolved position without needing separate left/right modes.
            if bounds.center().x < anchor.center().x {
                bounds.origin.x += bounds.size.width - width;
            }
            bounds.size.width = width;
            return ContentMask { bounds };
        }
        let from_bottom = match self.popup_origin {
            Some(PopupOrigin::Bottom) => true,
            Some(PopupOrigin::Anchor(anchor)) => bounds.center().y < anchor.center().y,
            _ => false,
        };
        let height = bounds.size.height * progress;
        if from_bottom {
            bounds.origin.y += bounds.size.height - height;
        }
        bounds.size.height = height;
        // The shared surface owns an inset border; child paint must never
        // escape its revealed bounds, including the final frame.
        ContentMask { bounds }
    }
}

impl IntoElement for Reveal {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Reveal {
    /// Whether the child takes part in the surrounding layout this frame.
    type RequestLayoutState = Flow;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Popup positioning sees the final size from the first frame. Only
        // its visible mask grows: the positioner cannot flip halfway open.
        if self.popup_origin.is_some() {
            return (self.child.request_layout(window, cx), Flow::Popup);
        }
        let Some(progress) = self.progress else {
            return if !self.expanded
                && let Some(preview) = &mut self.preview
            {
                (preview.request_layout(window, cx), Flow::Preview)
            } else {
                (self.child.request_layout(window, cx), Flow::Content)
            };
        };
        let content = self.child.request_layout(window, cx);
        let mut rows = Vec::with_capacity(2);
        if let Some(preview) = &mut self.preview {
            let preview = preview.request_layout(window, cx);
            rows.push(fractional_row(preview, 1. - progress, window, cx));
        }
        rows.push(fractional_row(content, progress, window, cx));
        let mut style = Style::default();
        style.display = Display::Flex;
        style.flex_direction = FlexDirection::Column;
        style.flex_shrink = 0.;
        style.size.width = relative(1.).into();
        (
            window.request_layout(style, rows, cx),
            Flow::Clipped {
                content,
                visible: false,
            },
        )
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        flow: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        match flow {
            Flow::Content => {
                self.child.prepaint(window, cx);
                return;
            }
            Flow::Preview => {
                self.preview
                    .as_mut()
                    .expect("Preview flow has a preview")
                    .prepaint(window, cx);
                return;
            }
            Flow::Clipped { content, visible } => {
                *visible = bounds.size.width > px(0.) && bounds.size.height > px(0.);
                if !*visible {
                    return;
                }
                // A preview reserves its own interpolated space but never
                // pushes the full content down. Both branches start at the
                // same origin, as at either settled endpoint.
                let content_bounds = window.layout_bounds(*content);
                let offset = bounds.origin - content_bounds.origin;
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    window.with_element_offset(offset, |window| self.child.prepaint(window, cx));
                });
            }
            Flow::Popup => {
                if self.progress.is_none() {
                    self.child.prepaint(window, cx);
                } else {
                    let mask = self.popup_mask(bounds);
                    window.with_content_mask(Some(mask), |window| self.child.prepaint(window, cx));
                }
                return;
            }
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        flow: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        match flow {
            Flow::Content => return self.child.paint(window, cx),
            Flow::Preview => {
                return self
                    .preview
                    .as_mut()
                    .expect("Preview flow has a preview")
                    .paint(window, cx);
            }
            Flow::Clipped { visible: false, .. } => return,
            Flow::Clipped { visible: true, .. } => {}
            Flow::Popup => {
                if self.progress.is_none() {
                    self.child.paint(window, cx);
                } else {
                    let mask = self.popup_mask(bounds);
                    window.with_content_mask(Some(mask), |window| self.child.paint(window, cx));
                }
                return;
            }
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.child.paint(window, cx);
        });
    }
}

/// The zero minimum allows the track to shrink; the block inside retains
/// its natural content layout and is clipped by the outer Reveal at paint.
fn fractional_row(content: LayoutId, fraction: f32, window: &mut Window, cx: &mut App) -> LayoutId {
    let mut holder = Style::default();
    holder.display = Display::Block;
    holder.min_size.width = px(0.).into();
    holder.min_size.height = px(0.).into();
    holder.overflow.y = Overflow::Hidden;
    let holder = window.request_layout(holder, [content], cx);
    let mut grid = Style::default();
    grid.display = Display::Grid;
    grid.grid_rows = Some(GridTemplate::fractional(1, fraction));
    grid.size.width = relative(1.).into();
    grid.flex_shrink = 0.;
    window.request_layout(grid, [holder], cx)
}

#[cfg(test)]
#[path = "reveal/layout_tests.rs"]
mod layout_tests;

#[cfg(test)]
mod tests {
    use super::{PopupOrigin, Reveal};
    use gpui_kit::{Bounds, IntoElement as _, div, point, px, size};

    #[::core::prelude::v1::test]
    fn popup_reveals_from_its_control_after_placement_flips() {
        let anchor = Bounds::new(point(px(20.), px(200.)), size(px(80.), px(24.)));
        let opening = Reveal::popup(
            "menu",
            PopupOrigin::Anchor(anchor),
            div().into_any_element(),
        )
        .with_progress(0.5);
        let below = Bounds::new(point(px(20.), px(228.)), size(px(200.), px(160.)));
        assert_eq!(
            opening.popup_mask(below).bounds,
            Bounds::new(point(px(20.), px(228.)), size(px(200.), px(80.)))
        );
        let above = Bounds::new(point(px(20.), px(36.)), size(px(200.), px(160.)));
        assert_eq!(
            opening.popup_mask(above).bounds,
            Bounds::new(point(px(20.), px(116.)), size(px(200.), px(80.)))
        );
        assert_eq!(opening.with_progress(1.).popup_mask(above).bounds, above);
    }

    #[::core::prelude::v1::test]
    fn popup_clip_is_empty_at_zero_and_continuous_at_rest() {
        let bounds = Bounds::new(point(px(20.), px(36.)), size(px(200.), px(160.)));
        for origin in [
            PopupOrigin::Anchor(Bounds::new(point(px(20.), px(0.)), size(px(80.), px(24.)))),
            PopupOrigin::Bottom,
        ] {
            let popup = |progress| {
                Reveal::popup("menu", origin, div().into_any_element())
                    .with_progress(progress)
                    .popup_mask(bounds)
                    .bounds
            };
            assert_eq!(popup(0.).size.height, px(0.));
            let almost = popup(0.999);
            let settled = popup(1.);
            assert_eq!(settled, bounds);
            assert_eq!(almost.left(), bounds.left());
            assert_eq!(almost.right(), bounds.right());
            assert!(almost.top() >= bounds.top());
            assert!(almost.bottom() <= bounds.bottom());
            assert!((settled.top() - almost.top()).abs() < px(0.2));
            assert!((settled.bottom() - almost.bottom()).abs() < px(0.2));
        }
    }

    #[::core::prelude::v1::test]
    fn horizontal_popup_reveals_from_the_panel_edge_at_zero_half_and_full() {
        let panel = Bounds::new(point(px(100.), px(36.)), size(px(20.), px(160.)));
        let left = Bounds::new(point(px(20.), px(36.)), size(px(80.), px(160.)));
        let right = Bounds::new(point(px(120.), px(36.)), size(px(80.), px(160.)));
        let mask = |bounds, progress| {
            Reveal::popup(
                "horizontal",
                PopupOrigin::Horizontal(panel),
                div().into_any_element(),
            )
            .with_progress(progress)
            .popup_mask(bounds)
            .bounds
        };

        assert_eq!(
            mask(left, 0.),
            Bounds::new(point(px(100.), px(36.)), size(px(0.), px(160.)))
        );
        assert_eq!(
            mask(left, 0.5),
            Bounds::new(point(px(60.), px(36.)), size(px(40.), px(160.)))
        );
        assert_eq!(mask(left, 1.), left);

        assert_eq!(
            mask(right, 0.),
            Bounds::new(point(px(120.), px(36.)), size(px(0.), px(160.)))
        );
        assert_eq!(
            mask(right, 0.5),
            Bounds::new(point(px(120.), px(36.)), size(px(40.), px(160.)))
        );
        assert_eq!(mask(right, 1.), right);
    }
}
