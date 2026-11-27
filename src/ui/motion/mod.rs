//! Shared motion of the client: one set of durations and easings for every
//! animation, so screens move alike.
//!
//! - `enter`: fades and rises of freshly shown blocks, staggered list rows;
//! - `disclosure`: chevrons and blocks that open and close;
//! - `collapse`: a measured horizontal reveal for strips of tabs;
//! - `transition`: a fade in natural layout when a stable host changes page;
//! - `number`: counters that glide to new values;
//! - `spinner`: the loading spinner;
//! - `ticker`: the shared low-rate clock of looping animations.
//!
//! Animation state lives with the element id, so an id that changes (another
//! page, a reopened section) plays the transition again.
//! Page transitions instead keep the host id stable and compare a page key.
//!
//! The motion preference is read on every call: «уменьшенные» keeps fades and
//! drops movement, «выключены» also sets gpui's reduce-motion flag so every
//! animation renders its final frame.

mod collapse;
mod disclosure;
mod enter;
mod number;
mod popup;
mod reveal;
mod shimmer;
mod slide;
mod spinner;
pub(super) mod ticker;
mod tooltip;
mod transition;
mod wave_dots;

pub(super) use collapse::Collapse;
pub(super) use disclosure::{
    chevron, drive, menu_chevron, moving, moving_at, preview_reveal, reveal, reveal_at,
    settle_disclosures, toggled,
};
pub(super) use enter::{fade_in_if, late_in, rise_in, rise_in_if, stagger_in, switch_in};
pub(super) use number::Number;
pub(super) use popup::{
    Popup, PopupPanel, popup, popup_inner_height, popup_inner_width, restore_focus,
};
pub(super) use reveal::PopupOrigin;
pub(super) use shimmer::Shimmer;
pub(super) use slide::{Slide, blend, glide};
pub(super) use spinner::Spinner;
pub(super) use tooltip::present as tooltip;
pub(super) use transition::PageTransition;
pub(super) use wave_dots::wave_dots;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use gpui_kit::component::animation::{ease_in_cubic, ease_out_cubic};
use gpui_kit::*;

use crate::preferences::MotionPreference;

/// Hover-sized changes: tabs appearing, small toggles.
pub(super) const FAST: Duration = Duration::from_millis(120);
/// Blocks appearing and panes taking their new places.
pub(super) const BASE: Duration = Duration::from_millis(180);
/// Larger content opening: disclosures, screens.
pub(super) const SLOW: Duration = Duration::from_millis(260);
/// A held pointer starts reordering only after an ordinary click has settled.
pub(super) const HOLD: Duration = Duration::from_millis(300);
/// Saved child history has no child stream; only visible active readers poll.
pub(super) const SAVED_CHAT_REFRESH: Duration = Duration::from_secs(1);
/// Backpressure and empty nonblocking responses yield instead of spinning.
pub(super) const TERMINAL_REFRESH: Duration = Duration::from_millis(50);

/// Easing of everything that arrives or moves to a new place.
pub(super) fn ease_enter(t: f32) -> f32 {
    ease_out_cubic(t)
}

/// Easing of everything that leaves.
pub(super) fn ease_exit(t: f32) -> f32 {
    ease_in_cubic(t)
}

pub(in crate::ui) fn register_window_globals(cx: &mut App) {
    disclosure::register_window_globals(cx);
}

/// Motion policy comes from the current execution window, never global focus.
pub(super) fn moves(cx: &App) -> bool {
    crate::ui::prefs::visual(cx).motion == MotionPreference::Full
}

pub(super) fn plays(cx: &App) -> bool {
    crate::ui::prefs::visual(cx).motion != MotionPreference::Off
}

fn shift(distance: f32, cx: &App) -> Pixels {
    px(if moves(cx) { distance } else { 0. })
}

/// A transition started at a moment; samples its progress over time.
#[derive(Clone, Copy, Debug)]
pub(super) struct Tween {
    at: Instant,
    duration: Duration,
}

impl Tween {
    pub(super) fn start(duration: Duration) -> Self {
        Self::start_at(duration, Instant::now())
    }

    pub(super) fn start_at(duration: Duration, at: Instant) -> Self {
        Self { at, duration }
    }

    /// Linear progress in `0..=1`.
    pub(super) fn linear(&self, now: Instant) -> f32 {
        let duration = self.duration.as_secs_f32();
        if duration <= 0. {
            return 1.;
        }
        (now.saturating_duration_since(self.at).as_secs_f32() / duration).min(1.)
    }

    pub(super) fn running(&self, now: Instant) -> bool {
        self.age(now) < self.duration
    }

    /// Time since the transition started.
    pub(super) fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.at)
    }
}

/// Keys that changed a moment ago: lets list rows animate only right after
/// they appeared, not every time they scroll back into view.
pub(super) struct Recent {
    marks: HashMap<u64, Instant>,
    window: Duration,
}

impl Default for Recent {
    fn default() -> Self {
        Self::with_window(Duration::from_millis(400))
    }
}

impl Recent {
    pub(super) fn with_window(window: Duration) -> Self {
        Self {
            marks: HashMap::new(),
            window,
        }
    }

    pub(super) fn mark(&mut self, key: u64) {
        let now = Instant::now();
        self.marks
            .retain(|_, at| now.duration_since(*at) < self.window);
        self.marks.insert(key, now);
    }

    pub(super) fn is_fresh(&self, key: u64) -> bool {
        self.marks
            .get(&key)
            .is_some_and(|at| at.elapsed() < self.window)
    }
}
