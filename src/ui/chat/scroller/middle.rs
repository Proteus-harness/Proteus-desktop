//! Browser-style, hold-only middle-button scrolling, explicitly requested for
//! the chat. Zed's editor/scroll/autoscroll.rs follows selections; it has no
//! matching middle-button mode. Keep this intentional difference local here.
use std::time::Instant;

use gpui_kit::component::*;
use gpui_kit::*;

use super::ChatScroller;
use crate::ui::prefs;

const MAX_SPEED: f32 = 6000.;
const MAX_FRAME_SECONDS: f32 = 1. / 30.;

#[derive(Default)]
pub(super) struct MiddleScroll {
    gesture: Option<Gesture>,
    escape: Option<Subscription>,
    activation: Option<Subscription>,
}

struct Gesture {
    origin: Point<Pixels>,
    pointer: Point<Pixels>,
    deadzone: Pixels,
    last_frame: Instant,
}

impl MiddleScroll {
    fn begin(&mut self, origin: Point<Pixels>, deadzone: Pixels, now: Instant) {
        self.gesture = Some(Gesture {
            origin,
            pointer: origin,
            deadzone,
            last_frame: now,
        });
    }

    pub(super) fn origin(&self) -> Option<Point<Pixels>> {
        self.gesture.as_ref().map(|gesture| gesture.origin)
    }

    fn moved(&mut self, pointer: Point<Pixels>, button: Option<MouseButton>) {
        if button != Some(MouseButton::Middle) {
            self.stop();
        } else if let Some(gesture) = &mut self.gesture {
            gesture.pointer = pointer;
        }
    }

    fn stop(&mut self) -> bool {
        self.escape = None;
        self.activation = None;
        self.gesture.take().is_some()
    }

    fn step(&mut self, now: Instant, focused: bool) -> Option<Pixels> {
        if !focused {
            self.stop();
            return None;
        }
        let gesture = self.gesture.as_mut()?;
        let dt = now
            .duration_since(gesture.last_frame)
            .as_secs_f32()
            .min(MAX_FRAME_SECONDS);
        gesture.last_frame = now;
        let offset = gesture.pointer.y - gesture.origin.y;
        let distance = f32::from((offset.abs() - gesture.deadzone).max(px(0.)));
        let speed = (distance * 12. + distance * distance * 0.12).min(MAX_SPEED);
        Some(px(f32::from(offset).signum() * speed * dt))
    }
}

impl ChatScroller {
    pub(super) fn begin_middle(
        &mut self,
        origin: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.wheel.clear(&self.list);
        self.hold = None;
        let deadzone = prefs::scaled(12.).to_pixels(window.rem_size());
        self.middle.begin(origin, deadzone, Instant::now());
        let owner = cx.weak_entity();
        let window_id = window.window_handle().window_id();
        // As with the native drag boundary, intercept before an editor or
        // popup maps Escape to another action. The subscription is gesture-only.
        self.middle.escape = Some(cx.intercept_keystrokes(move |event, window, cx| {
            if event.keystroke.key == "escape" && window.window_handle().window_id() == window_id {
                let stopped = owner
                    .update(cx, |state, cx| state.stop_middle(cx))
                    .unwrap_or(false);
                if stopped {
                    cx.stop_propagation();
                }
            }
        }));
        self.middle.activation = Some(cx.observe_window_activation(window, |state, window, cx| {
            if !window.is_window_active() {
                state.stop_middle(cx);
            }
        }));
        cx.notify();
    }

    pub(super) fn stop_middle(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.middle.stop() {
            return false;
        }
        self.wheel.clear(&self.list);
        cx.notify();
        true
    }

    pub(super) fn move_middle(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        if self.middle.origin().is_none() {
            return false;
        }
        self.middle.moved(event.position, event.pressed_button);
        if self.middle.origin().is_none() {
            self.wheel.clear(&self.list);
        }
        cx.notify();
        true
    }

    pub(super) fn step_middle(
        &mut self,
        now: Instant,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.middle.origin().is_none() {
            return false;
        }
        let Some(distance) = self.middle.step(now, focused) else {
            self.wheel.clear(&self.list);
            cx.notify();
            return false;
        };
        if distance != px(0.)
            && let Some(true) = self.user_scroll(point(px(0.), -distance))
        {
            cx.emit(super::HistorySeekCanceled);
        }
        true
    }
}

pub(super) fn indicator(
    origin: Point<Pixels>,
    viewport: Point<Pixels>,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let diameter = prefs::scaled(28.);
    let radius = diameter.to_pixels(window.rem_size()) / 2.;
    v_flex()
        .absolute()
        .left(origin.x - viewport.x - radius)
        .top(origin.y - viewport.y - radius)
        .w(diameter)
        .h(diameter)
        .items_center()
        .justify_center()
        .rounded(cx.theme().radius_full())
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().popover)
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(IconName::ChevronUp).xsmall())
        .child(Icon::new(IconName::ChevronDown).xsmall())
}

/// Global capture during an active hold, including release outside the list.
/// An inactive list adds no hitbox and does not consume any input.
pub(super) fn boundary(state: Entity<ChatScroller>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let release = state.clone();
            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase.capture()
                    && event.button == MouseButton::Middle
                    && release.read(cx).middle.origin().is_some()
                    && release.update(cx, |state, cx| state.stop_middle(cx))
                {
                    window.prevent_default();
                    cx.stop_propagation();
                }
            });
            let other = state.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, _, cx| {
                if phase.capture()
                    && event.button != MouseButton::Middle
                    && other.read(cx).middle.origin().is_some()
                {
                    other.update(cx, |state, cx| state.stop_middle(cx));
                }
            });
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                if phase.capture()
                    && state.read(cx).middle.origin().is_some()
                    && state.update(cx, |state, cx| state.move_middle(event, cx))
                {
                    cx.stop_propagation();
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

pub(super) fn next_frame(state: Entity<ChatScroller>, window: &mut Window) {
    window.on_next_frame(move |_, cx| {
        state.update(cx, |state, cx| {
            if state.middle.origin().is_some() {
                cx.notify();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::MiddleScroll;
    use gpui_kit::{MouseButton, point, px};
    use std::time::{Duration, Instant};

    #[test]
    fn a_click_or_small_pointer_wobble_does_not_scroll_or_latch() {
        let now = Instant::now();
        let mut middle = MiddleScroll::default();
        middle.begin(point(px(100.), px(100.)), px(12.), now);
        middle.moved(point(px(130.), px(110.)), Some(MouseButton::Middle));
        assert_eq!(
            middle.step(now + Duration::from_millis(16), true),
            Some(px(0.))
        );
        assert!(middle.stop());
        assert_eq!(middle.step(now + Duration::from_secs(1), true), None);
    }

    #[test]
    fn speed_follows_direction_and_distance_with_bounded_delayed_frames() {
        let now = Instant::now();
        let mut middle = MiddleScroll::default();
        middle.begin(point(px(0.), px(0.)), px(12.), now);
        middle.moved(point(px(0.), px(50.)), Some(MouseButton::Middle));
        let near = middle.step(now + Duration::from_millis(16), true).unwrap();
        middle.moved(point(px(0.), px(200.)), Some(MouseButton::Middle));
        let far = middle.step(now + Duration::from_millis(32), true).unwrap();
        assert!(far > near && near > px(0.));
        middle.moved(point(px(0.), px(-10000.)), Some(MouseButton::Middle));
        let delayed = middle.step(now + Duration::from_secs(60), true).unwrap();
        assert!(delayed < px(0.) && delayed.abs() <= px(200.01));
    }

    #[test]
    fn lost_middle_button_or_window_focus_stops_the_hold() {
        let now = Instant::now();
        let mut middle = MiddleScroll::default();
        middle.begin(point(px(0.), px(0.)), px(12.), now);
        middle.moved(point(px(0.), px(100.)), None);
        assert!(middle.origin().is_none());
        middle.begin(point(px(0.), px(0.)), px(12.), now);
        assert_eq!(middle.step(now + Duration::from_millis(16), false), None);
        assert!(middle.origin().is_none());
    }
}
