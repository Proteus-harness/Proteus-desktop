//! One clock for looping animations — spinners, status dots and shimmer. Each
//! of them asking for frames on its own cadence would add up to a redraw on
//! nearly every display frame; on a shared tick they all advance in the same
//! frame, at a rate that still reads as motion.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use gpui_kit::*;

/// Ticks per second while the window is active, and while it is not.
const ACTIVE_FPS: f32 = 30.;
const IDLE_FPS: f32 = 8.;

thread_local! {
    static PENDING: Cell<bool> = const { Cell::new(false) };
    static WAITING: RefCell<Vec<EntityId>> = const { RefCell::new(Vec::new()) };
}

/// Where a loop of `period` is now, from 0 to 1; loops started at different
/// times stay in step.
pub(in crate::ui) fn phase(period: Duration) -> f32 {
    static STARTED: OnceLock<Instant> = OnceLock::new();
    phase_at(STARTED.get_or_init(Instant::now).elapsed(), period)
}

fn phase_at(elapsed: Duration, period: Duration) -> f32 {
    // Keep whole turns out of f32: at long uptimes they swallow frame-sized steps.
    let fraction = (elapsed.as_secs_f64() / period.as_secs_f64()).fract() as f32;
    // Rounding the last fraction to 1.0 is the start of the next turn.
    if fraction == 1. { 0. } else { fraction }
}

/// Redraws the view being drawn on the next tick.
pub(in crate::ui) fn next_tick(window: &mut Window, cx: &mut App) {
    let view = window.current_view();
    WAITING.with_borrow_mut(|waiting| {
        if !waiting.contains(&view) {
            waiting.push(view);
        }
    });
    if PENDING.replace(true) {
        return;
    }
    let fps = if window.is_window_active() {
        ACTIVE_FPS
    } else {
        IDLE_FPS
    };
    window
        .spawn(cx, async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f32(1. / fps))
                .await;
            PENDING.set(false);
            let views = WAITING.take();
            cx.update(|_, cx| {
                for view in views {
                    cx.notify(view);
                }
            })
            .ok();
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::{Duration, phase_at};

    #[::core::prelude::v1::test]
    fn a_long_running_ticker_still_resolves_quarters_of_a_turn() {
        let period = Duration::from_millis(1_200);
        for days in [0, 7, 30, 120] {
            let start = Duration::from_secs(days * 86_400);
            for quarter in 0..4 {
                let t = start + Duration::from_millis(quarter * 300);
                assert!((phase_at(t, period) - quarter as f32 / 4.).abs() < 1e-6);
            }
        }
        let almost = period - Duration::from_nanos(1);
        assert!((0.0..1.0).contains(&phase_at(almost, period)));
    }
}
