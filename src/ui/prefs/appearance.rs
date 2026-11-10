// Адаптировано из Zed: crates/workspace/src/workspace.rs
//! System appearance arrives asynchronously on Linux. Track the window's
//! actual appearance and apply it only while the preference follows the system.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use super::{ClientPreferences, Effects, ThemePreference};

struct SystemAppearance(WindowAppearance);
impl Global for SystemAppearance {}

pub(super) fn current(cx: &App) -> WindowAppearance {
    cx.try_global::<SystemAppearance>()
        .map_or_else(|| cx.window_appearance(), |appearance| appearance.0)
}

/// Keep one subscription for each native window, including detached tabs.
/// No preference write is produced by a platform notification.
pub(in crate::ui) fn observe(window: &mut Window, cx: &mut App) -> Subscription {
    synchronize(window.appearance(), cx);
    window.observe_window_appearance(|window, cx| synchronize(window.appearance(), cx))
}

fn synchronize(appearance: WindowAppearance, cx: &mut App) {
    cx.set_global(SystemAppearance(appearance));
    let preferences = &cx.global::<ClientPreferences>().value;
    if preferences.theme != ThemePreference::System
        || Theme::global(cx).mode == ThemeMode::from(appearance)
    {
        return;
    }
    let preferences = preferences.clone();
    // Theme::change reloads the selected palette, so reapply the same scale,
    // font and radius policy as an explicit theme selection. Other effects
    // (renderer epochs, bindings and persistence) remain untouched.
    super::apply(
        &preferences,
        Effects {
            theme: true,
            refresh: true,
            ..Effects::default()
        },
        cx,
    );
}

#[cfg(test)]
#[path = "appearance_tests.rs"]
mod tests;
