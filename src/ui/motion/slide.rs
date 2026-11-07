//! A reveal that eases itself open or shut, so a row or a column can part
//! smoothly where something is about to land.

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::{Collapse, FAST};

/// Shows its child at full width (or height, when vertical) when `open`,
/// gliding there from where the previous frames left it. The first frame
/// shows the requested state at once.
#[derive(IntoElement)]
pub(in crate::ui) struct Slide {
    id: ElementId,
    open: bool,
    child: AnyElement,
    vertical: bool,
}

struct SlideState {
    progress: f32,
    at: Option<Instant>,
}

impl Slide {
    pub(in crate::ui) fn new(
        id: impl Into<ElementId>,
        open: bool,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            open,
            child: child.into_any_element(),
            vertical: false,
        }
    }

    pub(in crate::ui) fn vertical(mut self, vertical: bool) -> Self {
        self.vertical = vertical;
        self
    }
}

impl RenderOnce for Slide {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let eased = glide(self.id.clone(), self.open, FAST, window, cx);
        Collapse::new(self.id, eased, self.child).vertical(self.vertical)
    }
}

/// Eased share (0..=1) of an opening that glides toward `open` across
/// frames over `duration`; the first frame under `id` returns the requested
/// state at once.
pub(in crate::ui) fn glide(
    id: ElementId,
    open: bool,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    progress(id, open, duration, super::moves(), window, cx)
}

/// Reversible opacity progress. Reduced motion keeps this fade, while disabled
/// animations or the system reduce-motion flag settle immediately. The first
/// frame under `id` shows the requested state, just like `glide`.
pub(in crate::ui) fn blend(
    id: ElementId,
    open: bool,
    duration: Duration,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    progress(id, open, duration, super::plays(), window, cx)
}

fn progress(
    id: ElementId,
    open: bool,
    duration: Duration,
    enabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let target = if open { 1. } else { 0. };
    let state = window.use_keyed_state(id, cx, |_, _| SlideState::new(target));
    let animate = enabled && !cx.reduce_motion();
    let now = Instant::now();
    let (progress, running) =
        state.update(cx, |state, _| state.sample(target, duration, now, animate));
    if running {
        window.request_animation_frame();
    }
    progress
}

impl SlideState {
    fn new(progress: f32) -> Self {
        Self { progress, at: None }
    }

    /// Keep the current position on reversal, and stop scheduling frames once
    /// settled. Bound a delayed frame's step so resuming a window stays smooth.
    fn sample(
        &mut self,
        target: f32,
        duration: Duration,
        now: Instant,
        animate: bool,
    ) -> (f32, bool) {
        if !animate || duration.is_zero() {
            self.progress = target;
        }
        if self.progress == target {
            self.at = None;
            return (target, false);
        }
        let elapsed = self.at.map_or(0., |at| {
            now.saturating_duration_since(at).as_secs_f32().min(0.05)
        });
        self.at = Some(now);
        let step = elapsed / duration.as_secs_f32();
        self.progress = if target > self.progress {
            (self.progress + step).min(target)
        } else {
            (self.progress - step).max(target)
        };
        let running = self.progress != target;
        if !running {
            self.at = None;
        }
        (
            self.progress * self.progress * (3. - 2. * self.progress),
            running,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::motion::BASE;

    #[::core::prelude::v1::test]
    fn reversal_keeps_the_visible_position_and_resumes_from_it() {
        let now = Instant::now();
        let mut state = SlideState::new(0.);
        assert_eq!(state.sample(1., BASE, now, true), (0., true));
        let opening = state.sample(1., BASE, now + BASE / 4, true).0;
        assert!(opening > 0. && opening < 1.);
        // A new target does not restart at an endpoint in the same frame.
        assert_eq!(state.sample(0., BASE, now + BASE / 4, true).0, opening);
        let closing = state.sample(0., BASE, now + BASE / 2, true).0;
        assert!(closing < opening);
        assert_eq!(state.sample(1., BASE, now + BASE / 2, true).0, closing);
        assert!(state.sample(1., BASE, now + BASE * 3 / 4, true).0 > closing);
    }

    #[::core::prelude::v1::test]
    fn completed_transitions_do_not_keep_requesting_frames() {
        let now = Instant::now();
        let mut state = SlideState::new(0.);
        assert_eq!(state.sample(0., BASE, now, true), (0., false));
        state.sample(1., BASE, now, true);
        for frame in 1..=10 {
            state.sample(1., BASE, now + BASE * frame / 4, true);
        }
        assert_eq!(state.sample(1., BASE, now + BASE * 3, true), (1., false));
        assert!(state.at.is_none());
    }

    #[::core::prelude::v1::test]
    fn disabling_motion_during_a_transition_settles_without_more_frames() {
        let now = Instant::now();
        let mut state = SlideState::new(0.);
        state.sample(1., BASE, now, true);
        state.sample(1., BASE, now + BASE / 4, true);
        assert_eq!(state.sample(1., BASE, now + BASE / 4, false), (1., false));
        assert_eq!(state.sample(0., BASE, now + BASE / 4, false), (0., false));
        assert!(state.at.is_none());
    }

    #[::core::prelude::v1::test]
    fn zero_duration_finishes_in_the_first_frame() {
        let mut state = SlideState::new(0.);
        assert_eq!(
            state.sample(1., Duration::ZERO, Instant::now(), true),
            (1., false)
        );
        assert!(state.at.is_none());
    }
}
