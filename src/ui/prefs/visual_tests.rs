//! Native-window state tests: changing visual settings never touches an agent.
use super::*;
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;
use std::sync::Arc;

fn initialize(cx: &mut App) {
    gpui_kit::init(cx);
    crate::ui::theme::init(cx);
    init_in_memory(cx);
}

fn add_window(app: &mut ::gpui::TestAppContext) -> ::gpui::VisualTestContext {
    let handle = app.add_empty_window().window_handle();
    ::gpui::VisualTestContext::from_window(handle, app)
}

#[::core::prelude::v1::test]
fn native_windows_keep_independent_visual_values_and_theme_projections() {
    let mut app = ::gpui::TestAppContext::single();
    app.update(initialize);
    let before = app.update(|cx| cx.global::<ClientPreferences>().value.clone());
    let mut first = add_window(&mut app);
    let mut second = add_window(&mut app);
    first.update(|window, cx| {
        initialize_visual_window(window, cx);
        update_visual(cx, |config| {
            config.theme = crate::preferences::ThemePreference::Dark;
            config.view.scale = 125;
            config.view.interface_text = 20;
            config.view.chat_text = 18;
            config.view.settings_width = 1000;
            config.extensions.set_enabled("plan", false);
            config.package_visual_settings.insert(
                "plan".into(),
                serde_json::from_value(serde_json::json!({"visibility":"always"})).unwrap(),
            );
        });
    });
    second.update(|window, cx| {
        initialize_visual_window(window, cx);
        update_visual(cx, |config| {
            config.theme = crate::preferences::ThemePreference::Light;
            config.view.scale = 90;
            config.view.settings_width = 600;
            config.motion = crate::preferences::MotionPreference::Off;
        });
    });
    app.run_until_parked();
    first.update(|_, cx| {
        assert_eq!(visual(cx).view.scale, 125);
        assert_eq!(settings(cx).view.interface_text, 20);
        assert_eq!(chat_text(cx), px(22.5));
        assert_eq!(chat_width(cx), px(1025.));
        assert_eq!(zoomed(visual(cx).view.settings_width, cx), px(1250.));
        assert_eq!(visual(cx).view.settings_width, 1000);
        assert_eq!(settings(cx).view.settings_width, 1000);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Dark);
        assert_eq!(Theme::global(cx).font_size, px(25.));
        assert_eq!(Theme::global(cx).mono_font_size, px(16.25));
        assert_eq!(Theme::global(cx).radius_lg, px(20.));
        assert!(!visual(cx).extensions.is_enabled("plan"));
        assert_eq!(
            visual(cx).package_visual_settings["plan"]["visibility"],
            "always"
        );
        assert!(!cx.reduce_motion());
    });
    let first_revision = first.update(|_, cx| visual_revision(cx));
    let first_renderer_epoch = first.update(|_, cx| crate::ui::extensions::render_generation(cx));
    second.update(|_, cx| {
        assert_ne!(visual_revision(cx), first_revision);
        assert_ne!(
            crate::ui::extensions::render_generation(cx),
            first_renderer_epoch
        );
        let old_window_values = serde_json::Map::new();
        assert!(update_package_visual("plan", first_revision, old_window_values, cx).is_err());
        assert_eq!(visual(cx).view.scale, 90);
        assert_eq!(
            settings(cx).view.interface_text,
            crate::preferences::DEFAULT_INTERFACE_TEXT
        );
        assert_eq!(visual(cx).view.settings_width, 600);
        assert_eq!(settings(cx).view.settings_width, 600);
        assert_eq!(Theme::global(cx).mode, ThemeMode::Light);
        assert!(visual(cx).extensions.is_enabled("plan"));
        assert!(visual(cx).package_visual_settings.is_empty());
        assert!(cx.reduce_motion());
    });
    app.update(|cx| {
        assert!(Arc::ptr_eq(
            &before,
            &cx.global::<ClientPreferences>().value
        ))
    });
}

#[::core::prelude::v1::test]
fn late_package_command_is_rejected_after_a_window_visual_change() {
    let mut app = ::gpui::TestAppContext::single();
    app.update(initialize);
    let mut window = add_window(&mut app);
    window.update(|window, cx| {
        initialize_visual_window(window, cx);
        let revision = visual_revision(cx);
        update_visual(cx, |config| config.view.scale = 125);
        let values = serde_json::from_value(serde_json::json!({"visibility":"always"})).unwrap();
        assert!(update_package_visual("plan", revision, values, cx).is_err());
        assert!(visual(cx).package_visual_settings.is_empty());
    });
}

#[::core::prelude::v1::test]
fn a_later_manual_load_wins_over_an_earlier_save_as_completion() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("build/visual-ordering-tests")
        .join(uuid::Uuid::new_v4().to_string());
    std::fs::create_dir_all(&root).unwrap();
    let first = root.join("a.json");
    let second = root.join("b.json");
    let mut config = crate::visual_config::VisualConfig::default();
    crate::visual_config::save(&first, &config, None).unwrap();
    config.view.scale = 125;
    crate::visual_config::save(&second, &config, None).unwrap();
    let mut app = ::gpui::TestAppContext::single();
    app.update(initialize);
    let mut window = add_window(&mut app);
    window.update(|window, cx| {
        initialize_visual_window(window, cx);
        let old_save_intent = visual_intent(cx);
        let old_package_revision = visual_revision(cx);
        // This starts a genuine async load and records its intent immediately.
        select_visual(second.clone(), window, cx);
        // An already-issued package command must not cancel the newer load.
        assert!(
            update_package_visual("plan", old_package_revision, serde_json::Map::new(), cx)
                .is_err()
        );
        assert!(!select_visual_if_current(
            old_save_intent,
            first.clone(),
            window,
            cx
        ));
    });
    app.run_until_parked();
    window.update(|_, cx| {
        assert_eq!(
            cx.global::<WindowVisualPreferences>().selection.path,
            Some(second)
        );
        assert_eq!(visual(cx).view.scale, 125);
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[::core::prelude::v1::test]
fn detached_visual_snapshot_inherits_once_then_changes_independently() {
    let mut app = ::gpui::TestAppContext::single();
    app.update(initialize);
    let mut source = add_window(&mut app);
    source.update(|window, cx| {
        initialize_visual_window(window, cx);
        update_visual(cx, |config| config.view.scale = 125);
    });
    let inherited = source.update(|_, cx| visual_snapshot(cx));
    let original_revision = inherited.revision;
    let mut detached = add_window(&mut app);
    detached.update(|_, cx| {
        inherit_visual_window(inherited, cx);
        assert_eq!(visual(cx).view.scale, 125);
        assert_ne!(visual_revision(cx), original_revision);
        update_visual(cx, |config| config.view.scale = 90);
    });
    app.run_until_parked();
    source.update(|_, cx| assert_eq!(visual(cx).view.scale, 125));
    detached.update(|_, cx| assert_eq!(visual(cx).view.scale, 90));
}

#[::core::prelude::v1::test]
fn chat_font_roles_remain_window_local_and_user_override_resets_to_agent() {
    let mut app = ::gpui::TestAppContext::single();
    app.update(initialize);
    let before = app.update(|cx| cx.global::<ClientPreferences>().value.clone());
    let mut first = add_window(&mut app);
    let mut second = add_window(&mut app);
    first.update(|window, cx| {
        initialize_visual_window(window, cx);
        update_visual(cx, |config| {
            config.fonts.chat = Some("Portable first agent".into());
            config.fonts.user_chat = Some("Portable first user".into());
        });
        assert_eq!(
            settings(cx).fonts.user_chat.as_deref(),
            Some("Portable first user")
        );
    });
    second.update(|window, cx| {
        initialize_visual_window(window, cx);
        update_visual(cx, |config| {
            config.fonts.chat = Some("Portable second agent".into());
            config.fonts.user_chat = None;
        });
        assert_eq!(user_chat_font(cx), chat_font(cx));
        assert!(settings(cx).fonts.user_chat.is_none());
    });
    first.update(|_, cx| {
        assert_eq!(
            visual(cx).fonts.chat.as_deref(),
            Some("Portable first agent")
        );
        assert_eq!(
            visual(cx).fonts.user_chat.as_deref(),
            Some("Portable first user")
        );
        update_settings(cx, |preferences| {
            crate::preferences::Preference::UserChatFont.reset(preferences)
        });
        assert!(visual(cx).fonts.user_chat.is_none());
        assert_eq!(
            visual(cx).fonts.chat.as_deref(),
            Some("Portable first agent")
        );
        assert_eq!(user_chat_font(cx), chat_font(cx));
    });
    second.update(|_, cx| {
        assert_eq!(
            visual(cx).fonts.chat.as_deref(),
            Some("Portable second agent")
        );
        assert!(visual(cx).fonts.user_chat.is_none());
    });
    app.update(|cx| {
        assert!(Arc::ptr_eq(
            &before,
            &cx.global::<ClientPreferences>().value
        ))
    });
}
