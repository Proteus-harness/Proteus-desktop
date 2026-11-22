//! A width control must not move its own track during a native pointer drag.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};

struct WidthControl {
    state: Entity<SliderState>,
    _subscription: Subscription,
}

impl Render for WidthControl {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .justify_end()
            .w(px(f32::from(prefs::visual(cx).view.settings_width)))
            .max_w_full()
            .h(px(32.))
            .child(slider(&self.state, "px", cx))
    }
}

fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[::core::prelude::v1::test]
fn settings_width_previews_without_moving_the_track_and_applies_on_release() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::theme::init(cx);
        prefs::init_in_memory(cx);
    });
    let (view, cx) = app.add_window_view(|window, cx| {
        let state = cx.new(|_| {
            SliderState::new()
                .max(f32::from(*SETTINGS_WIDTH.end()))
                .min(f32::from(*SETTINGS_WIDTH.start()))
                .step(10.)
                .default_value(720.)
        });
        let subscription = follow_on_release(&state, window, cx, |prefs, value| {
            prefs.view.settings_width = value
        });
        WidthControl {
            state,
            _subscription: subscription,
        }
    });
    cx.update(|window, _| window.activate_window());
    draw(cx);
    let state = view.read_with(cx, |view, _| view.state.clone());
    let bounds = cx.update(|_, cx| state.read(cx).bounds());
    let at = |fraction| {
        point(
            bounds.left() + bounds.size.width * fraction,
            bounds.center().y,
        )
    };
    let thumb = at((720. - 420.) / (1600. - 420.));
    cx.simulate_mouse_move(thumb, None, Modifiers::none());
    cx.simulate_mouse_down(thumb, MouseButton::Left, Modifiers::none());
    // The first move starts GPUI's native drag, the next moves its thumb.
    for fraction in [0.5, 0.6, 0.7] {
        cx.simulate_mouse_move(at(fraction), Some(MouseButton::Left), Modifiers::none());
        draw(cx);
    }
    let pending = cx.update(|_, cx| {
        let state = state.read(cx);
        assert!(
            state.value().end() > 1000.,
            "the readout must preview the drag"
        );
        assert_eq!(
            state.bounds(),
            bounds,
            "preview must not move or resize the track"
        );
        assert_eq!(prefs::visual(cx).view.settings_width, 720);
        state.value().end() as u16
    });
    cx.simulate_mouse_up(at(0.7), MouseButton::Left, Modifiers::none());
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(prefs::visual(cx).view.settings_width, pending);
        assert_ne!(state.read(cx).bounds().origin, bounds.origin);
    });

    // A track click still applies as soon as it ends; it does not wait for
    // another drag or a settings save action.
    let bounds = cx.update(|_, cx| state.read(cx).bounds());
    let target = point(bounds.left() + bounds.size.width * 0.1, bounds.center().y);
    cx.simulate_mouse_down(target, MouseButton::Left, Modifiers::none());
    draw(cx);
    cx.update(|_, cx| assert_eq!(prefs::visual(cx).view.settings_width, pending));
    cx.simulate_mouse_up(target, MouseButton::Left, Modifiers::none());
    draw(cx);
    cx.update(|_, cx| {
        assert_eq!(prefs::visual(cx).view.settings_width, 540);
        assert_eq!(state.read(cx).value().end(), 540.);
    });
}
