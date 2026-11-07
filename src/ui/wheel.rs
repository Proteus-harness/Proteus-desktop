//! Smooth scrolling. A notched wheel arrives as whole lines and gpui
//! scrolls them in one jump; the window root takes those events and replays
//! them as pixel steps over a few frames, so whatever scrolls under the
//! pointer glides as in a browser. Touchpad scrolling comes in pixels and
//! passes through, then keeps gliding after a flick (`fling`). Nothing is
//! smoothed when motion is off. Holding Ctrl scrolls several times faster.

mod fling;

use std::cell::{Cell, RefCell};
use std::time::Instant;

use gpui_kit::*;

use super::motion;

/// Time constant of the glide: the remaining distance shrinks by ~63% per τ,
/// so a notch settles in about 0.2 s.
const GLIDE_SECONDS: f32 = 0.07;
const SETTLED: Pixels = px(0.5);
/// How much faster scrolling goes while Ctrl is held.
const FAST: f32 = 5.;

#[derive(Default)]
struct Glide {
    window: Option<WindowId>,
    generation: u64,
    remaining: Point<Pixels>,
    modifiers: Modifiers,
    last_frame: Option<Instant>,
    running: bool,
}

thread_local! {
    static GLIDE: RefCell<Glide> = RefCell::default();
    /// Set while this layer dispatches its own steps, which pass through it.
    static REPLAYING: Cell<bool> = const { Cell::new(false) };
}

/// An invisible layer for the window root that turns wheel lines into a glide,
/// lets touchpad flicks coast and speeds up any scrolling while Ctrl is held.
/// It goes before the content: capture handlers run in paint order, and a
/// scroller that consumes the wheel there (gpui-kit's `ScrollableMask` over
/// the chat list) would otherwise hide the gesture from the layer.
pub(super) fn layer() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |_, _, window, _| {
            window.on_mouse_event(|_: &MouseDownEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    fling::stop();
                    if super::kit::keyboard_input(false) {
                        cx.refresh_windows();
                    }
                    super::tip::dismiss(window, cx);
                }
            });
            window.on_mouse_event(|event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture {
                    super::tip::moved(event.position, window, cx);
                }
            });
            window.on_mouse_event(|event: &ScrollEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                super::tip::dismiss(window, cx);
                if REPLAYING.get() {
                    return;
                }
                // GPUI owns touch pans and their momentum; this layer only
                // smooths wheel notches and pointer/trackpad gestures.
                if event.source == ScrollSource::Touch {
                    fling::stop();
                    return;
                }
                let fast = event.modifiers.control;
                // Replayed events go out without Ctrl.
                let modifiers = Modifiers {
                    control: false,
                    ..event.modifiers
                };
                let speed = if fast { FAST } else { 1. };
                match event.delta {
                    ScrollDelta::Pixels(pixels) if coasts() => {
                        fling::track(event.position, pixels * speed, modifiers, window);
                    }
                    _ => fling::stop(),
                }
                let lines = matches!(event.delta, ScrollDelta::Lines(_));
                let glide = lines && motion::plays();
                if !glide && !fast {
                    return;
                }
                cx.stop_propagation();
                if !glide {
                    let delta = match event.delta {
                        ScrollDelta::Lines(lines) => ScrollDelta::Lines(lines * speed),
                        ScrollDelta::Pixels(pixels) => ScrollDelta::Pixels(pixels * speed),
                    };
                    let replayed = ScrollEvent {
                        delta,
                        modifiers,
                        ..event.clone()
                    };
                    window.defer(cx, move |window, cx| replay(replayed, window, cx));
                    return;
                }
                let delta = event.delta.pixel_delta(window.line_height()) * speed;
                let start = GLIDE.with_borrow_mut(|glide| {
                    if glide.window != Some(window.window_handle().window_id()) {
                        *glide = Glide {
                            window: Some(window.window_handle().window_id()),
                            generation: glide.generation.wrapping_add(1),
                            ..Default::default()
                        };
                    }
                    glide.remaining += delta;
                    glide.modifiers = modifiers;
                    !std::mem::replace(&mut glide.running, true)
                });
                if start {
                    GLIDE.with_borrow_mut(|glide| glide.last_frame = Some(Instant::now()));
                    let generation = GLIDE.with_borrow(|glide| glide.generation);
                    window.on_next_frame(move |window, cx| step(window, cx, generation));
                }
            });
        },
    )
    .absolute()
    .size_full()
}

/// Touchpad inertia is added where the platform has none of its own.
fn coasts() -> bool {
    cfg!(target_os = "linux") && motion::plays()
}

/// Dispatches a scroll step of this layer past its own handler.
fn replay(event: ScrollEvent, window: &mut Window, cx: &mut App) {
    REPLAYING.set(true);
    window.dispatch_event(PlatformInput::Scroll(event), cx);
    REPLAYING.set(false);
}

/// Scrolls by this frame's share of the remaining distance.
fn step(window: &mut Window, cx: &mut App, generation: u64) {
    if GLIDE.with_borrow(|glide| {
        glide.window != Some(window.window_handle().window_id()) || glide.generation != generation
    }) {
        return;
    }
    let now = Instant::now();
    let (delta, modifiers, more) = GLIDE.with_borrow_mut(|glide| {
        let dt = glide.last_frame.map_or(1. / 60., |at| {
            now.duration_since(at).as_secs_f32().min(0.05)
        });
        glide.last_frame = Some(now);
        let share = 1. - (-dt / GLIDE_SECONDS).exp();
        let mut delta = point(glide.remaining.x * share, glide.remaining.y * share);
        let left = glide.remaining - delta;
        if left.x.abs() < SETTLED && left.y.abs() < SETTLED {
            delta = glide.remaining;
            glide.remaining = Point::default();
            glide.running = false;
        } else {
            glide.remaining = left;
        }
        (delta, glide.modifiers, glide.running)
    });
    replay(
        ScrollEvent {
            position: window.mouse_position(),
            delta: ScrollDelta::Pixels(delta),
            modifiers,
            touch_phase: TouchPhase::Moved,
            source: ScrollSource::Pointer,
        },
        window,
        cx,
    );
    if more {
        window.on_next_frame(move |window, cx| step(window, cx, generation));
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn stale_window_and_generation_callbacks_cannot_move_the_new_glide() {
        let mut cx = ::gpui::TestAppContext::single();
        let a = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        let b = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        GLIDE.with_borrow_mut(|glide| {
            *glide = Glide {
                window: Some(b.window_id()),
                generation: 2,
                remaining: point(px(0.), px(200.)),
                running: true,
                ..Default::default()
            }
        });
        cx.update_window(a, |_, window, cx| step(window, cx, 1))
            .unwrap();
        cx.update_window(b, |_, window, cx| step(window, cx, 1))
            .unwrap();
        assert_eq!(GLIDE.with_borrow(|glide| glide.remaining.y), px(200.));
        GLIDE.with_borrow_mut(|glide| *glide = Glide::default());
    }
}
