//! A stable page host fades new content in at its natural size.

use std::time::Instant;

use gpui_kit::*;

use super::{FAST, Tween, ease_enter};

struct PageState {
    key: SharedString,
    fade: Option<Tween>,
}

impl PageState {
    fn new(key: SharedString) -> Self {
        Self { key, fade: None }
    }

    fn sample(&mut self, key: SharedString, now: Instant, play: bool) -> (f32, bool) {
        if self.key != key {
            self.key = key;
            self.fade = play.then(|| Tween::start_at(FAST, now));
        }
        if let Some(fade) = self.fade.filter(|fade| play && fade.running(now)) {
            return (ease_enter(fade.linear(now)), true);
        }
        self.fade = None;
        (1., false)
    }
}

#[derive(IntoElement)]
pub(in crate::ui) struct PageTransition {
    id: ElementId,
    key: SharedString,
    child: AnyElement,
}

impl PageTransition {
    pub fn new(id: impl Into<ElementId>, key: impl Into<SharedString>, child: AnyElement) -> Self {
        Self {
            id: id.into(),
            key: key.into(),
            child,
        }
    }
}

impl RenderOnce for PageTransition {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let play = super::plays(cx) && !cx.reduce_motion();
        let now = Instant::now();
        let state = window.use_keyed_state((self.id.clone(), "fade"), cx, |_, _| {
            PageState::new(self.key.clone())
        });
        let (opacity, running) = state.update(cx, |state, _| state.sample(self.key, now, play));
        if running {
            window.request_animation_frame();
        }
        // Use the parent's ordinary layout pass. A measured height tween
        // changes the scroll range and repeats layout across multiple frames.
        div()
            .id(self.id)
            .min_w_0()
            .opacity(opacity)
            .child(self.child)
    }
}

#[cfg(test)]
mod tests {
    use super::{FAST, PageState};
    use std::time::Instant;

    #[::core::prelude::v1::test]
    fn only_a_page_change_starts_a_fade() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into());
        assert_eq!(state.sample("summary".into(), now, true), (1., false));
        assert_eq!(state.sample("models".into(), now, true), (0., true));
        let (opacity, running) = state.sample("models".into(), now + FAST / 2, true);
        assert!(running && opacity > 0. && opacity < 1.);
        assert_eq!(state.sample("models".into(), now + FAST, true), (1., false));
    }

    #[::core::prelude::v1::test]
    fn disabling_motion_finishes_a_running_fade_without_replaying_it() {
        let now = Instant::now();
        let mut state = PageState::new("summary".into());
        state.sample("models".into(), now, true);
        assert_eq!(
            state.sample("models".into(), now + FAST / 2, false),
            (1., false)
        );
        assert_eq!(
            state.sample("models".into(), now + FAST / 2, true),
            (1., false)
        );
    }
}
