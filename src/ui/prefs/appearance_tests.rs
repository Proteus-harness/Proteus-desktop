use std::sync::Arc;

use ::gpui::TestAppContext;
use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};
use gpui_kit::*;

use super::super::{ClientPreferences, ThemePreference, init_in_memory, update};

fn initialize(cx: &mut App, preference: ThemePreference) {
    gpui_kit::init(cx);
    crate::ui::theme::init(cx);
    init_in_memory(cx);
    ThemeRegistry::global_mut(cx)
        .load_themes_from_str(
            r#"{"name":"Appearance fixture","themes":[
                {"name":"Appearance fixture dark","mode":"dark","font.family":"Fixture Dark UI"},
                {"name":"Appearance fixture light","mode":"light","font.family":"Fixture Light UI"}
            ]}"#,
        )
        .unwrap();
    update(cx, |preferences| {
        preferences.theme = preference;
        preferences.themes.dark = "Appearance fixture dark".into();
        preferences.themes.light = "Appearance fixture light".into();
        preferences.view.scale = 125;
        preferences.view.surface_radius = 18;
    });
}

fn assert_mode(mode: ThemeMode, cx: &App) {
    let theme = Theme::global(cx);
    assert_eq!(theme.mode, mode);
    assert_eq!(
        theme.theme_name().as_ref(),
        if mode.is_dark() {
            "Appearance fixture dark"
        } else {
            "Appearance fixture light"
        }
    );
    assert_eq!(
        theme.font_family.as_ref(),
        if mode.is_dark() {
            "Fixture Dark UI"
        } else {
            "Fixture Light UI"
        }
    );
    assert_eq!(theme.font_size, px(20.));
    assert_eq!(theme.mono_font_size, px(16.25));
    assert_eq!(theme.radius, px(11.25));
    assert_eq!(theme.radius_lg, px(22.5));
    let base = gpui_kit::base::Theme::global(cx);
    assert_eq!(
        base.appearance,
        if mode.is_dark() {
            gpui_kit::base::ThemeAppearance::Dark
        } else {
            gpui_kit::base::ThemeAppearance::Light
        }
    );
}

#[::core::prelude::v1::test]
fn late_system_appearance_changes_theme_without_writing_preferences_or_changing_metrics() {
    let mut cx = TestAppContext::single();
    cx.update(|cx| initialize(cx, ThemePreference::System));
    let window = cx.add_empty_window();
    let _subscription = window.update(|window, cx| super::observe(window, cx));
    let saved = cx.update(|cx| {
        assert_mode(ThemeMode::Light, cx);
        cx.global::<ClientPreferences>().value.clone()
    });
    let renderer_generation = cx.update(|cx| crate::ui::extensions::render_generation(cx));
    for (appearance, mode) in [
        (WindowAppearance::Dark, ThemeMode::Dark),
        (WindowAppearance::Light, ThemeMode::Light),
    ] {
        // GPUI exposes native appearance injection only to its own crate.
        // Inject at the app callback boundary after startup, keeping the real
        // theme/prefs/Base projections and the deferred delivery order.
        cx.update(|cx| cx.defer(move |cx| super::synchronize(appearance, cx)));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_mode(mode, cx);
            assert!(Arc::ptr_eq(&saved, &cx.global::<ClientPreferences>().value));
            assert_eq!(cx.global::<ClientPreferences>().revision, 0);
            assert_eq!(
                crate::ui::extensions::render_generation(cx),
                renderer_generation
            );
        });
    }
}

#[::core::prelude::v1::test]
fn first_window_observation_uses_its_actual_appearance_instead_of_a_stale_app_sample() {
    let mut cx = TestAppContext::single();
    cx.update(|cx| initialize(cx, ThemePreference::System));
    let window = cx.add_empty_window();
    let _subscription = window.update(|window, cx| {
        super::synchronize(WindowAppearance::Dark, cx);
        assert_mode(ThemeMode::Dark, cx);
        super::observe(window, cx)
    });
    cx.update(|cx| assert_mode(ThemeMode::Light, cx));
}

#[::core::prelude::v1::test]
fn fixed_modes_ignore_platform_changes_but_returning_to_system_uses_latest_window_appearance() {
    for (preference, fixed, system, resulting) in [
        (
            ThemePreference::Dark,
            ThemeMode::Dark,
            WindowAppearance::Light,
            ThemeMode::Light,
        ),
        (
            ThemePreference::Light,
            ThemeMode::Light,
            WindowAppearance::Dark,
            ThemeMode::Dark,
        ),
    ] {
        let mut cx = TestAppContext::single();
        cx.update(|cx| initialize(cx, preference));
        let window = cx.add_empty_window();
        let _subscription = window.update(|window, cx| super::observe(window, cx));
        cx.update(|cx| cx.defer(move |cx| super::synchronize(system, cx)));
        cx.run_until_parked();
        cx.update(|cx| {
            assert_mode(fixed, cx);
            update(cx, |preferences| {
                preferences.theme = ThemePreference::System
            });
            assert_mode(resulting, cx);
        });
    }
}
