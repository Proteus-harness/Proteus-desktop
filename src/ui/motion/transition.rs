//! A stable page host fades new content in. Floating pages also interpolate
//! the surface height through the shared fractional reveal layout.

use std::{cell::Cell, rc::Rc, time::Instant};

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{FAST, SLOW, Tween, ease_enter, reveal::Reveal};

struct PageState {
    key: SharedString,
    fade: Option<Tween>,
    visible_height: Rc<Cell<Pixels>>,
    from: Pixels,
}

impl PageState {
    fn new(key: SharedString) -> Self {
        Self {
            key,
            fade: None,
            visible_height: Rc::default(),
            from: px(0.),
        }
    }

    fn sample(&mut self, key: SharedString, now: Instant, play: bool, resize: bool) -> f32 {
        if self.key != key {
            self.key = key;
            self.from = self.visible_height.get();
            let duration = if resize && self.from > px(0.) {
                SLOW
            } else {
                FAST
            };
            self.fade = play.then(|| Tween::start_at(duration, now));
        }
        if let Some(fade) = self.fade.filter(|fade| play && fade.running(now)) {
            return ease_enter(fade.linear(now));
        }
        self.fade = None;
        1.
    }
}

#[derive(IntoElement)]
pub(in crate::ui) struct PageTransition {
    id: ElementId,
    key: SharedString,
    child: AnyElement,
    resize: bool,
}

impl PageTransition {
    pub fn new(id: impl Into<ElementId>, key: impl Into<SharedString>, child: AnyElement) -> Self {
        Self {
            id: id.into(),
            key: key.into(),
            child,
            resize: false,
        }
    }

    pub fn resize(mut self, resize: bool) -> Self {
        self.resize = resize;
        self
    }
}

impl RenderOnce for PageTransition {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let play = super::plays(cx) && !cx.reduce_motion();
        let resize = self.resize && super::moves(cx);
        let now = Instant::now();
        let state = window.use_keyed_state((self.id.clone(), "fade"), cx, |_, _| {
            PageState::new(self.key.clone())
        });
        let (progress, from, measured) = state.update(cx, |state, _| {
            (
                state.sample(self.key, now, play, resize),
                state.from,
                state.visible_height.clone(),
            )
        });
        if progress < 1. {
            window.request_animation_frame();
        }
        let content = div()
            .min_w_0()
            .opacity(progress)
            .child(self.child)
            .into_any_element();
        let content = if self.resize {
            let id = (self.id.clone(), "extent");
            let reveal = if resize && from > px(0.) && progress < 1. {
                // Keep only the previous height. Both tracks share the current
                // parent layout, so the new page needs no separate measurement.
                Reveal::preview(
                    id,
                    Some(progress),
                    true,
                    div().w_full().h(from).into_any_element(),
                    content,
                )
            } else {
                // Keep control ids stable between the animated and settled layouts.
                Reveal::new(id, None, content)
            };
            reveal.into_any_element()
        } else {
            content
        };
        // The exit point of an interrupted transition is the height that
        // actually reached prepaint, including the current host constraints.
        div()
            .when(self.resize, |page| {
                page.on_children_prepainted(move |bounds, _, _| {
                    if let Some(bounds) = bounds.first() {
                        measured.set(bounds.size.height);
                    }
                })
            })
            .id(self.id)
            .min_w_0()
            .child(content)
    }
}

#[cfg(test)]
mod tests {
    use super::{FAST, PageState, SLOW, px};
    use std::time::Instant;

    #[::core::prelude::v1::test]
    fn only_a_page_change_starts_a_fade() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into());
        assert_eq!(state.sample("summary".into(), now, true, false), 1.);
        assert_eq!(state.sample("models".into(), now, true, false), 0.);
        let opacity = state.sample("models".into(), now + FAST / 2, true, false);
        assert!(opacity > 0. && opacity < 1.);
        assert_eq!(state.sample("models".into(), now + FAST, true, false), 1.);
    }

    #[::core::prelude::v1::test]
    fn disabling_motion_finishes_a_running_fade_without_replaying_it() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into());
        state.sample("models".into(), now, true, true);
        assert_eq!(
            state.sample("models".into(), now + FAST / 2, false, false),
            1.
        );
        assert_eq!(
            state.sample("models".into(), now + FAST / 2, true, true),
            1.
        );
    }

    #[::core::prelude::v1::test]
    fn a_popup_page_change_starts_from_its_last_visible_height() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into());
        state.visible_height.set(px(56.));
        assert_eq!(state.sample("models".into(), now, true, true), 0.);
        assert_eq!(state.from, px(56.));
        let midway = now + SLOW / 2;
        assert!(state.sample("models".into(), midway, true, true) < 1.);
        state.visible_height.set(px(179.));
        assert_eq!(state.sample("summary".into(), midway, true, true), 0.);
        assert_eq!(state.from, px(179.));
    }
}
