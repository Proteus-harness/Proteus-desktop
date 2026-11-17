//! Sliders for visual sizes. Settings width commits on release so its own
//! controls stay under the pointer while the value is being chosen.

#[cfg(test)]
mod tests;

use gpui_kit::component::h_flex;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::*;

use super::SettingsView;
use crate::preferences::{
    CHAT_TEXT, CHAT_WIDTH, Preference, Preferences, SETTINGS_WIDTH, SURFACE_RADIUS,
};
use crate::ui::prefs;

pub(super) struct SizeSliders {
    pub chat_text: Entity<SliderState>,
    pub chat_width: Entity<SliderState>,
    pub settings_width: Entity<SliderState>,
    pub surface_radius: Entity<SliderState>,
}

impl SizeSliders {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> (Self, Vec<Subscription>) {
        let view = prefs::visual(cx).view;
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
        let settings_width = cx.new(|_| {
            SliderState::new()
                .max(f32::from(*SETTINGS_WIDTH.end()))
                .min(f32::from(*SETTINGS_WIDTH.start()))
                .step(10.)
                .default_value(f32::from(view.settings_width))
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
            follow_settings_width(&settings_width, window, cx),
        ];
        (
            Self {
                chat_text,
                chat_width,
                surface_radius,
                settings_width,
            },
            subscriptions,
        )
    }
    pub fn synchronize(&self, window: &mut Window, cx: &mut Context<SettingsView>) {
        let view = prefs::visual(cx).view;
        for (state, value) in [
            (&self.chat_text, view.chat_text),
            (&self.chat_width, view.chat_width),
            (&self.surface_radius, view.surface_radius),
            (&self.settings_width, view.settings_width),
        ] {
            let value = f32::from(value);
            if state.read(cx).value().end() != value {
                state.update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
    }

    pub fn reset(&self, field: Preference, window: &mut Window, cx: &mut Context<SettingsView>) {
        let defaults = crate::preferences::ViewPreferences::default();
        let (state, value) = match field {
            Preference::SurfaceRadius => (&self.surface_radius, defaults.surface_radius),
            Preference::ChatText => (&self.chat_text, defaults.chat_text),
            Preference::ChatWidth => (&self.chat_width, defaults.chat_width),
            Preference::SettingsWidth => (&self.settings_width, defaults.settings_width),
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
        prefs::update_settings(cx, |prefs| apply(prefs, value));
        cx.notify();
    })
}

/// Resizing Settings also moves this slider, so only its readout and thumb
/// preview the pending value. Track clicks and accessibility steps release
/// immediately; a drag applies and persists once the pointer is released.
fn follow_settings_width<V: 'static>(
    slider: &Entity<SliderState>,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Subscription {
    cx.subscribe_in(slider, window, move |_, _, event: &SliderEvent, _, cx| {
        if let SliderEvent::Release(value) = event {
            let value = value.end().round() as u16;
            prefs::update_settings(cx, |prefs| prefs.view.settings_width = value);
        }
        cx.notify();
    })
}

/// The slider with its current value on the right.
pub(super) fn slider(state: &Entity<SliderState>, unit: &'static str, cx: &App) -> AnyElement {
    let state_value = state.read(cx);
    let value = state_value.value().end().round() as u16;
    let digits = (state_value.max_value().round() as u16).to_string().len();
    h_flex()
        .w(prefs::scaled(240.))
        .min_w_0()
        .max_w_full()
        .gap_2()
        .child(div().flex_1().min_w_0().child(Slider::new(state).compact()))
        .child(
            div()
                .relative()
                .flex_none()
                .text_sm()
                .text_right()
                .font_features(FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])))
                // Reserve the full readout width: crossing 999 -> 1000 must
                // not shrink the track underneath an active drag.
                .child(
                    div()
                        .invisible()
                        .child(format!("{} {unit}", "0".repeat(digits))),
                )
                .child(
                    div()
                        .absolute()
                        .right_0()
                        .top_0()
                        .child(format!("{value} {unit}")),
                ),
        )
        .into_any_element()
}
