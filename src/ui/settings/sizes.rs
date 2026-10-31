//! Sliders for chat text size and width; each move applies and stores it.

use gpui_kit::component::h_flex;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::*;

use super::SettingsView;
use crate::preferences::{CHAT_TEXT, CHAT_WIDTH, Preference, Preferences, SURFACE_RADIUS};
use crate::ui::prefs;

pub(super) struct SizeSliders {
    pub chat_text: Entity<SliderState>,
    pub chat_width: Entity<SliderState>,
    pub surface_radius: Entity<SliderState>,
}

impl SizeSliders {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> (Self, Vec<Subscription>) {
        let view = prefs::current(cx).view;
        let chat_text = cx.new(|_| {
            SliderState::new()
                .max(f32::from(*CHAT_TEXT.end()))
                .min(f32::from(*CHAT_TEXT.start()))
                .step(1.)
                .default_value(f32::from(view.chat_text))
        });
        let chat_width = cx.new(|_| {
            SliderState::new()
                .max(f32::from(*CHAT_WIDTH.end()))
                .min(f32::from(*CHAT_WIDTH.start()))
                .step(10.)
                .default_value(f32::from(view.chat_width))
        });
        let surface_radius = cx.new(|_| {
            SliderState::new()
                .max(f32::from(*SURFACE_RADIUS.end()))
                .min(f32::from(*SURFACE_RADIUS.start()))
                .step(1.)
                .default_value(f32::from(view.surface_radius))
        });
        let subscriptions = vec![
            follow(&chat_text, window, cx, |prefs, value| {
                prefs.view.chat_text = value
            }),
            follow(&chat_width, window, cx, |prefs, value| {
                prefs.view.chat_width = value
            }),
            follow(&surface_radius, window, cx, |prefs, value| {
                prefs.view.surface_radius = value
            }),
        ];
        (
            Self {
                chat_text,
                chat_width,
                surface_radius,
            },
            subscriptions,
        )
    }
    pub fn reset(&self, field: Preference, window: &mut Window, cx: &mut Context<SettingsView>) {
        let defaults = crate::preferences::ViewPreferences::default();
        let (state, value) = match field {
            Preference::SurfaceRadius => (&self.surface_radius, defaults.surface_radius),
            Preference::ChatText => (&self.chat_text, defaults.chat_text),
            Preference::ChatWidth => (&self.chat_width, defaults.chat_width),
            _ => return,
        };
        state.update(cx, |state, cx| {
            state.set_value(f32::from(value), window, cx)
        });
    }
}

fn follow(
    slider: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
    apply: fn(&mut Preferences, u16),
) -> Subscription {
    cx.subscribe_in(slider, window, move |_, _, event: &SliderEvent, _, cx| {
        let (SliderEvent::Change(value) | SliderEvent::Release(value)) = event;
        let value = value.end().round() as u16;
        prefs::update(cx, |prefs| apply(prefs, value));
        cx.notify();
    })
}

/// The slider with its current value on the right.
pub(super) fn slider(state: &Entity<SliderState>, unit: &'static str, cx: &App) -> AnyElement {
    let value = state.read(cx).value().end().round() as u16;
    h_flex()
        .w(prefs::scaled(240.))
        .min_w_0()
        .max_w_full()
        .gap_2()
        .child(div().flex_1().min_w_0().child(Slider::new(state).compact()))
        .child(
            div()
                .flex_none()
                .text_sm()
                .text_right()
                .child(format!("{value} {unit}")),
        )
        .into_any_element()
}
