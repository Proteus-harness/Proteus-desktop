//! The element under every disclosure. While the disclosure rests open the
//! child takes part in the surrounding layout and its height is remembered,
//! so a close starts from that height instead of collapsing to nothing for a
//! frame. While it moves, the child is laid out on its own and clipped to a
//! share of its height.

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
    Anchor(Bounds<Pixels>),
}

#[derive(Clone, Copy, Default)]
struct RevealState {
    height: Option<Pixels>,
    preview_height: Option<Pixels>,
}

pub(super) enum Flow {
    Content,
    Preview,
    Clipped,
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
        let from_bottom = match self.popup_origin {
            Some(PopupOrigin::Bottom) => true,
            Some(PopupOrigin::Anchor(anchor)) => bounds.center().y < anchor.center().y,
            None => false,
        };
        let height = bounds.size.height * self.progress.unwrap_or(1.);
        if from_bottom {
            bounds.origin.y += bounds.size.height - height;
        }
        bounds.size.height = height;
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
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Popup positioning sees the final size from the first frame. Only
        // its visible mask grows: the positioner cannot flip halfway open.
        if self.popup_origin.is_some() && self.progress.is_some() {
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
        let measured = window.with_element_state(
            global_id.expect("Reveal has an id"),
            |state: Option<RevealState>, _| {
                let state = state.unwrap_or_default();
                (state, state)
            },
        );
        // A block never seen at rest has no height yet: one that starts
        // closing shows in full for the frame that measures it, one that
        // starts opening shows nothing.
        let Some(height) = measured.height else {
            if self.preview.is_none() && progress >= 0.5 {
                return (self.child.request_layout(window, cx), Flow::Content);
            }
            let mut style = Style::default();
            style.size.width = relative(1.).into();
            style.size.height = measured.preview_height.unwrap_or_default().into();
            return (window.request_layout(style, None, cx), Flow::Clipped);
        };
        let minimum = measured.preview_height.unwrap_or_default().min(height);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (minimum + (height - minimum) * progress).into();
        (window.request_layout(style, None, cx), Flow::Clipped)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        flow: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let id = global_id.expect("Reveal has an id");
        match flow {
            Flow::Content => {
                remember(id, bounds.size.height, false, window);
                if let Some(preview) = &mut self.preview {
                    let measured = preview.layout_as_root(
                        size(
                            AvailableSpace::Definite(bounds.size.width),
                            AvailableSpace::MinContent,
                        ),
                        window,
                        cx,
                    );
                    remember(id, measured.height, true, window);
                }
                self.child.prepaint(window, cx);
                return;
            }
            Flow::Preview => {
                remember(id, bounds.size.height, true, window);
                self.preview
                    .as_mut()
                    .expect("Preview flow has a preview")
                    .prepaint(window, cx);
                return;
            }
            Flow::Clipped => {}
            Flow::Popup => {
                let mask = self.popup_mask(bounds);
                window.with_content_mask(Some(mask), |window| self.child.prepaint(window, cx));
                return;
            }
        }
        let measured = self.child.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::MinContent,
            ),
            window,
            cx,
        );
        let mut changed = remember(id, measured.height, false, window);
        if let Some(preview) = &mut self.preview {
            let measured = preview.layout_as_root(
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::MinContent,
                ),
                window,
                cx,
            );
            changed |= remember(id, measured.height, true, window);
        }
        if changed {
            window.request_animation_frame();
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.child.prepaint_at(bounds.origin, window, cx);
        });
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
            Flow::Clipped => {}
            Flow::Popup => {
                let mask = self.popup_mask(bounds);
                window.with_content_mask(Some(mask), |window| self.child.paint(window, cx));
                return;
            }
        }
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            self.child.paint(window, cx);
        });
    }
}

/// Stores the child's height; true when it differs from the stored one.
fn remember(id: &GlobalElementId, height: Pixels, preview: bool, window: &mut Window) -> bool {
    window.with_element_state(id, |state: Option<RevealState>, _| {
        let mut state = state.unwrap_or_default();
        let previous = if preview {
            &mut state.preview_height
        } else {
            &mut state.height
        };
        let changed = *previous != Some(height);
        *previous = Some(height);
        (changed, state)
    })
}

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
            Bounds::new(point(px(20.), px(228.)), size(px(200.), px(80.)),)
        );
        let above = Bounds::new(point(px(20.), px(36.)), size(px(200.), px(160.)));
        assert_eq!(
            opening.popup_mask(above).bounds,
            Bounds::new(point(px(20.), px(116.)), size(px(200.), px(80.)),)
        );
        assert_eq!(opening.with_progress(1.).popup_mask(above).bounds, above);
    }
}
