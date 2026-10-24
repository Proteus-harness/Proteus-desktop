//! Measured page changes: a stable host keeps its current height when the
//! page key changes. The new page lays out in full while its host resizes
//! and fades it in; ordinary updates of the same page keep natural layout.

use std::time::Instant;

use gpui_kit::*;

use super::{SLOW, ease_enter};

#[derive(Clone)]
struct Change {
    from: Pixels,
    to: Pixels,
    at: Instant,
}

impl Change {
    fn progress(&self, now: Instant) -> f32 {
        ease_enter(
            (now.saturating_duration_since(self.at).as_secs_f32() / SLOW.as_secs_f32()).min(1.),
        )
    }

    fn height(&self, now: Instant) -> Pixels {
        self.from + (self.to - self.from) * self.progress(now)
    }
}

#[derive(Clone)]
struct PageState {
    key: SharedString,
    size: Size<Pixels>,
    change: Option<Change>,
}

impl PageState {
    fn new(key: SharedString, size: Size<Pixels>) -> Self {
        Self {
            key,
            size,
            change: None,
        }
    }

    fn prepare(&mut self, key: &SharedString, now: Instant, play: bool) {
        if self.key != *key {
            let from = self
                .change
                .as_ref()
                .map_or(self.size.height, |change| change.height(now));
            self.key = key.clone();
            self.change = play.then_some(Change {
                from,
                to: self.size.height,
                at: now,
            });
        } else if !play
            || self
                .change
                .as_ref()
                .is_some_and(|change| now.saturating_duration_since(change.at) >= SLOW)
        {
            self.change = None;
        }
    }

    fn measure(&mut self, size: Size<Pixels>) {
        self.size = size;
        if let Some(change) = &mut self.change {
            change.to = size.height;
        }
    }
}

pub(in crate::ui) struct PageTransition {
    id: ElementId,
    key: SharedString,
    child: Option<AnyElement>,
    drawn: Option<AnyElement>,
}

impl PageTransition {
    pub fn new(id: impl Into<ElementId>, key: impl Into<SharedString>, child: AnyElement) -> Self {
        Self {
            id: id.into(),
            key: key.into(),
            child: Some(child),
            drawn: None,
        }
    }
}

impl IntoElement for PageTransition {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

pub(in crate::ui) enum PageFlow {
    Natural,
    Changing { resize: bool },
}

impl Element for PageTransition {
    type RequestLayoutState = PageFlow;
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
    ) -> (LayoutId, PageFlow) {
        let id = global_id.expect("PageTransition has an id");
        let now = Instant::now();
        let mut state = window.with_element_state(id, |state: Option<PageState>, _| {
            let state = state.unwrap_or_else(|| PageState::new(self.key.clone(), Size::default()));
            (state.clone(), state)
        });
        state.prepare(&self.key, now, super::plays() && !cx.reduce_motion());
        let change = state.change.as_ref();
        let opacity = change.map_or(1., |change| change.progress(now));
        let mut child = div()
            .min_w_0()
            .opacity(opacity)
            .child(self.child.take().expect("PageTransition lays out once"))
            .into_any_element();
        let (layout, flow) = if state.change.is_some() {
            let natural = child.layout_as_root(
                size(
                    AvailableSpace::Definite(state.size.width),
                    AvailableSpace::MaxContent,
                ),
                window,
                cx,
            );
            state.measure(natural);
            let resize = super::moves();
            let height = if resize {
                state.change.as_ref().unwrap().height(now)
            } else {
                natural.height
            };
            let mut style = Style::default();
            style.size.width = relative(1.).into();
            style.size.height = height.into();
            style.flex_shrink = 0.;
            window.request_animation_frame();
            (
                window.request_layout(style, None, cx),
                PageFlow::Changing { resize },
            )
        } else {
            (child.request_layout(window, cx), PageFlow::Natural)
        };
        self.drawn = Some(child);
        window.with_element_state(id, |_: Option<PageState>, _| ((), state));
        (layout, flow)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        flow: &mut PageFlow,
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = self.drawn.as_mut().expect("PageTransition was laid out");
        let natural = match flow {
            PageFlow::Natural => bounds.size,
            PageFlow::Changing { .. } => child.layout_as_root(
                size(
                    AvailableSpace::Definite(bounds.size.width),
                    AvailableSpace::MaxContent,
                ),
                window,
                cx,
            ),
        };
        window.with_element_state(
            global_id.expect("PageTransition has an id"),
            |state: Option<PageState>, _| {
                let mut state = state.unwrap_or_else(|| PageState::new(self.key.clone(), natural));
                state.measure(natural);
                ((), state)
            },
        );
        match flow {
            PageFlow::Natural => {
                child.prepaint(window, cx);
            }
            PageFlow::Changing { resize: false } => {
                child.prepaint_at(bounds.origin, window, cx);
            }
            PageFlow::Changing { resize: true } => {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    child.prepaint_at(bounds.origin, window, cx);
                })
            }
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        flow: &mut PageFlow,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = self.drawn.as_mut().expect("PageTransition was laid out");
        if matches!(flow, PageFlow::Changing { resize: true }) {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                child.paint(window, cx)
            });
        } else {
            child.paint(window, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PageState, SLOW};
    use gpui_kit::{px, size};
    use std::time::Instant;

    #[::core::prelude::v1::test]
    fn first_mount_is_natural_and_key_changes_start_at_the_visible_height() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into(), size(px(280.), px(124.)));
        state.prepare(&"summary".into(), now, true);
        assert!(state.change.is_none());
        state.prepare(&"models".into(), now, true);
        state.measure(size(px(280.), px(306.)));
        assert_eq!(state.change.as_ref().unwrap().height(now), px(124.));
        let middle = now + SLOW / 2;
        let shown = state.change.as_ref().unwrap().height(middle);
        assert!(shown > px(124.) && shown < px(306.));
        state.prepare(&"summary".into(), middle, true);
        state.measure(size(px(280.), px(124.)));
        assert_eq!(state.change.as_ref().unwrap().height(middle), shown);
        assert_eq!(
            state.change.as_ref().unwrap().height(middle + SLOW),
            px(124.)
        );
    }

    #[::core::prelude::v1::test]
    fn same_key_updates_measure_current_content_without_restarting_the_transition() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into(), size(px(280.), px(124.)));
        state.prepare(&"models".into(), now, true);
        state.measure(size(px(280.), px(306.)));
        state.prepare(&"models".into(), now + SLOW / 2, true);
        state.measure(size(px(280.), px(180.)));
        assert_eq!(state.change.as_ref().unwrap().at, now);
        assert_eq!(state.change.as_ref().unwrap().to, px(180.));
        state.prepare(&"models".into(), now + SLOW, true);
        assert!(state.change.is_none());
        state.measure(size(px(280.), px(240.)));
        assert_eq!(state.size.height, px(240.));
        assert!(state.change.is_none());
    }

    #[::core::prelude::v1::test]
    fn off_or_system_reduced_motion_snaps_a_running_transition() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into(), size(px(280.), px(124.)));
        state.prepare(&"models".into(), now, true);
        state.measure(size(px(280.), px(306.)));
        state.prepare(&"models".into(), now + SLOW / 2, false);
        assert!(state.change.is_none());
        assert_eq!(state.size.height, px(306.));
    }
}
