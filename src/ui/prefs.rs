//! Client preferences in the running app: a global plus the effects of each
//! setting (theme, motion level).

mod appearance;
mod persistence;
mod visual;
#[cfg(test)]
mod visual_tests;
pub(in crate::ui) use visual::{
    WindowVisualPreferences, current as visual, inherit as inherit_visual_window,
    initialize as initialize_visual_window, intent as visual_intent,
    package_update as update_package_visual, report as visual_error,
    reset_auto as reset_visual_auto, reset_default as reset_visual_default,
    revision as visual_revision, select as select_visual,
    select_if_current as select_visual_if_current, set_agent as set_visual_agent,
    snapshot as visual_snapshot, update as update_visual,
};

pub(super) use appearance::observe as observe_appearance;

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::preferences::{self, MotionPreference, Preferences, ThemePreference};

const BASE_FONT: f32 = 16.;
const BASE_MONO_FONT: f32 = 13.;
/// Interface scale: 1.0 at 100%.
pub fn scale(cx: &App) -> f32 {
    visual(cx).view.scale_factor()
}

/// A length given in px at 100% scale; follows the interface scale.
pub fn scaled(value: f32) -> Rems {
    rems(value / BASE_FONT)
}

/// A laid out length in the units of [`scaled`].
pub fn unscaled(length: Pixels, window: &Window) -> f32 {
    f32::from(length) / f32::from(window.rem_size()) * BASE_FONT
}

/// Text size of chat messages and the composer.
pub fn chat_text(cx: &App) -> Rems {
    scaled(f32::from(visual(cx).view.chat_text))
}

/// Conversation prose and composer family; code retains the theme's mono.
pub fn chat_font(cx: &App) -> SharedString {
    visual::chat_font(cx).unwrap_or_else(|| Theme::global(cx).font_family.clone())
}

/// Widest conversation column.
pub fn chat_width(cx: &App) -> Rems {
    scaled(f32::from(visual(cx).view.chat_width))
}

pub struct ClientPreferences {
    pub value: Arc<Preferences>,
    writer: Option<persistence::Writer<Preferences>>,
    revision: u64,
    /// Why the stored file was not used; shown on the client settings pages.
    pub problem: Option<String>,
    persistent: bool,
}

impl Global for ClientPreferences {}

/// Headless entity regressions need defaults, not a disk writer or changes to
/// the user's process-global config environment.
#[cfg(test)]
pub(in crate::ui) fn init_in_memory(cx: &mut App) {
    cx.set_global(ClientPreferences {
        value: Arc::new(Preferences::default()),
        writer: None,
        revision: 0,
        problem: None,
        persistent: false,
    });
    visual::init(cx);
}

pub fn init(cx: &mut App, loaded: anyhow::Result<Preferences>) {
    let (value, mut problem) = match loaded {
        Ok(value) => (value, None),
        Err(error) => (Preferences::default(), Some(format!("{error:#}"))),
    };
    let (writer, receiver) = match persistence::Writer::new(|value| {
        preferences::save(value).map_err(|error| format!("{error:#}"))
    }) {
        Ok((writer, receiver)) => (Some(writer), Some(receiver)),
        Err(error) => {
            problem = Some(format!("Не удалось запустить сохранение настроек: {error}"));
            (None, None)
        }
    };
    cx.set_global(ClientPreferences {
        value: Arc::new(value),
        problem,
        persistent: true,
        writer,
        revision: 0,
    });
    let value = cx.global::<ClientPreferences>().value.clone();
    apply(&value, Effects::initial(), cx);
    visual::init(cx);
    if let Some(receiver) = receiver {
        cx.spawn(async move |cx| {
            while receiver.recv().await.is_ok() {
                let _ = cx.update(|cx| {
                    let global = cx.global::<ClientPreferences>();
                    let problem = global
                        .writer
                        .as_ref()
                        .and_then(|writer| writer.completion())
                        .and_then(|completion| completion.problem_for(global.revision));
                    if let Some(problem) = problem {
                        if global.problem != problem {
                            cx.global_mut::<ClientPreferences>().problem = problem;
                            cx.refresh_windows();
                        }
                    }
                });
            }
        })
        .detach();
        cx.on_app_quit(|cx| {
            let shutdown = cx
                .global::<ClientPreferences>()
                .writer
                .as_ref()
                .map(|writer| writer.shutdown());
            // GPUI allows only 200 ms for async quit futures. Wait here, with
            // an explicit upper bound, so delayed disk I/O does not lose the
            // last selection. Normal edits never wait for the filesystem.
            if let Some(shutdown) = shutdown {
                if !shutdown.wait(Duration::from_secs(3)) {
                    eprintln!("preferences: timed out waiting for the last save");
                }
            }
            async {}
        })
        .detach();
    }
}

pub fn current(cx: &App) -> &Preferences {
    &cx.global::<ClientPreferences>().value
}

/// A small settings projection: visual fields are window-owned; input policy
/// remains device-wide. Session histories/catalog metadata are never cloned.
pub(in crate::ui) fn settings(cx: &App) -> Preferences {
    let mut value = Preferences::default();
    visual(cx).apply_to_preferences(&mut value);
    let device = current(cx);
    value.send = device.send;
    value.notifications = device.notifications;
    value.shortcuts = device.shortcuts.clone();
    value
}

pub(in crate::ui) fn update_settings(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    let mut value = settings(cx);
    change(&mut value);
    let mut visual_value = crate::visual_config::VisualConfig::from_preferences(&value);
    visual_value.package_visual_settings = visual(cx).package_visual_settings.clone();
    update_visual(cx, |current| *current = visual_value);
    update(cx, |device| {
        device.send = value.send;
        device.notifications = value.notifications;
        device.shortcuts = value.shortcuts;
    });
}

/// Applies the change right away and stores it on this device.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    let previous = &cx.global::<ClientPreferences>().value;
    let mut value = (**previous).clone();
    change(&mut value);
    if value == **previous {
        return;
    }
    let effects = Effects::between(previous, &value);
    let visual_defaults_changed = previous.theme != value.theme
        || previous.themes != value.themes
        || previous.fonts != value.fonts
        || previous.motion != value.motion
        || previous.hints != value.hints
        || previous.label_truncation != value.label_truncation
        || previous.view != value.view
        || previous.extensions != value.extensions
        || previous.sidebar != value.sidebar;
    if visual_defaults_changed && cx.current_execution_window_id().is_none() {
        visual::update_application_default(&value, cx);
    }

    let value = Arc::new(value);
    let submitted = cx
        .global::<ClientPreferences>()
        .writer
        .as_ref()
        .map(|writer| writer.submit(value.clone()));
    let global = cx.global_mut::<ClientPreferences>();
    global.value = value.clone();
    match submitted {
        Some(Ok(revision)) => global.revision = revision,
        Some(Err(error)) => {
            global.revision = global.revision.saturating_add(1);
            global.problem = Some(format!("Не сохранено: {error}"));
        }
        None => {}
    }
    apply(&value, effects, cx);
}

/// Compute from borrowed values before mutating App; session metadata never
/// rebinds keys, reapplies a theme or invalidates every window.
#[derive(Clone, Copy, Debug, Default)]
struct Effects {
    theme: bool,
    sizes: bool,
    shortcuts: bool,
    motion: bool,
    chat_font: bool,
    renderers: bool,
    refresh: bool,
}

impl Effects {
    fn initial() -> Self {
        Self {
            theme: true,
            sizes: true,
            shortcuts: true,
            motion: true,
            chat_font: true,
            renderers: true,
            refresh: true,
        }
    }

    fn between(previous: &Preferences, value: &Preferences) -> Self {
        Self {
            theme: previous.theme != value.theme
                || previous.themes != value.themes
                || previous.fonts.interface != value.fonts.interface,
            sizes: previous.view.scale != value.view.scale
                || previous.view.surface_radius != value.view.surface_radius,
            shortcuts: previous.shortcuts != value.shortcuts,
            motion: previous.motion != value.motion,
            chat_font: previous.fonts.chat != value.fonts.chat,
            renderers: previous.extensions.disabled != value.extensions.disabled
                || previous.extensions.order != value.extensions.order,
            refresh: previous.theme != value.theme
                || previous.themes != value.themes
                || previous.fonts != value.fonts
                || previous.motion != value.motion
                || previous.view != value.view
                || previous.shortcuts != value.shortcuts
                || previous.hints != value.hints
                || previous.label_truncation != value.label_truncation
                || previous.extensions != value.extensions
                || previous.catalogs != value.catalogs
                || previous.visual_configs != value.visual_configs
                || previous.sidebar != value.sidebar
                || previous.send != value.send
                || previous.notifications != value.notifications,
        }
    }
}

fn apply(value: &Preferences, effects: Effects, cx: &mut App) {
    if effects.renderers {
        super::extensions::bump_render_generation(cx);
    }
    if effects.theme {
        super::theme::select(&value.themes, value.fonts.interface.as_deref(), cx);
        match value.theme {
            ThemePreference::System => Theme::change(appearance::current(cx), None, cx),
            ThemePreference::Dark => Theme::change(ThemeMode::Dark, None, cx),
            ThemePreference::Light => Theme::change(ThemeMode::Light, None, cx),
        }
        super::theme::project_interactions(cx);
    }
    // The root sets the window's rem from the theme font size, so this
    // scales every rem-based size of the interface.
    // Corners follow the same scale.
    if effects.sizes || effects.theme {
        let scale = value.view.scale_factor();
        let theme = Theme::global(cx);
        let surface_radius = px(f32::from(value.view.surface_radius) * scale);
        let control_radius = surface_radius / 2.;
        if theme.font_size != px(BASE_FONT * scale)
            || theme.radius != control_radius
            || theme.radius_lg != surface_radius
        {
            Theme::update(cx, |theme| {
                theme.font_size = px(BASE_FONT * scale);
                theme.mono_font_size = px(BASE_MONO_FONT * scale);
                theme.radius = control_radius;
                theme.radius_lg = surface_radius;
            });
        }
    }
    if effects.shortcuts {
        super::keys::bind(&value.shortcuts, cx);
    }
    if effects.motion {
        cx.set_reduce_motion(value.motion == MotionPreference::Off);
    }
    if effects.refresh {
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn session_layout_and_model_metadata_do_not_invalidate_all_windows() {
        let before = Preferences::default();
        let mut after = before.clone();
        after.preferred_agent = crate::conversations::AgentKind::Codex;
        after.tabs.select("codex:thread-fixture");
        after.tabs_workspace = Some(crate::preferences::TabWorkspace {
            workspace: "/fixture".into(),
            profile: None,
        });
        after.sessions.rename(
            &crate::tabs::ChatKey::Codex("thread-fixture".into()),
            "Renamed",
        );
        after.recent.touch(std::path::Path::new("/fixture"));
        let effects = Effects::between(&before, &after);
        assert!(!effects.refresh);
        assert!(!effects.theme);
        assert!(!effects.sizes);
        assert!(!effects.shortcuts);
        assert!(!effects.motion);
        assert!(!effects.chat_font);
        assert!(!effects.renderers);
    }

    #[::core::prelude::v1::test]
    fn visual_registry_refreshes_controls_without_reapplying_window_appearance() {
        let before = Preferences::default();
        let mut after = before.clone();
        after.visual_configs.default_config = Some("/fixture/desktop.json".into());
        let effects = Effects::between(&before, &after);
        assert!(effects.refresh);
        assert!(!effects.theme && !effects.sizes && !effects.motion && !effects.chat_font);
        assert!(!effects.shortcuts && !effects.renderers);
        after
            .visual_configs
            .bindings
            .insert("/fixture/agent.toml".into(), "/fixture/desktop.json".into());
        assert!(Effects::between(&before, &after).refresh);
    }

    #[::core::prelude::v1::test]
    fn appearance_changes_retain_refresh_and_the_required_effects() {
        let before = Preferences::default();
        let changes: Vec<Box<dyn Fn(&mut Preferences)>> = vec![
            Box::new(|p| p.theme = ThemePreference::Light),
            Box::new(|p| p.fonts.interface = Some("Fixture UI".into())),
            Box::new(|p| p.fonts.chat = Some("Fixture Chat".into())),
            Box::new(|p| p.view.scale = 125),
            Box::new(|p| p.view.surface_radius = 4),
            Box::new(|p| p.view.chat_text = 20),
            Box::new(|p| p.motion = MotionPreference::Off),
            Box::new(|p| p.hints.description = false),
            Box::new(|p| p.extensions.disabled.push("fixture".into())),
            Box::new(|p| p.label_truncation = preferences::LabelTruncation::Ellipsis),
        ];
        for change in changes {
            let mut after = before.clone();
            change(&mut after);
            assert!(Effects::between(&before, &after).refresh);
        }
        let mut after = before.clone();
        after.fonts.chat = Some("Fixture Chat".into());
        let effects = Effects::between(&before, &after);
        assert!(effects.chat_font);
        assert!(!effects.theme);
        after.fonts.interface = Some("Fixture UI".into());
        assert!(Effects::between(&before, &after).theme);
        after.view.scale = 125;
        assert!(Effects::between(&before, &after).sizes);
        after.motion = MotionPreference::Off;
        assert!(Effects::between(&before, &after).motion);
    }

    #[::core::prelude::v1::test]
    fn content_renderer_selection_changes_its_epoch_but_badge_placement_does_not() {
        let cx = ::gpui::TestAppContext::single();
        cx.update(|cx| {
            gpui_kit::init(cx);
            init_in_memory(cx);
            let initial = super::super::extensions::render_generation(cx);
            update(cx, |preferences| {
                preferences
                    .extensions
                    .set_enabled("fixture-renderer", false)
            });
            let disabled = super::super::extensions::render_generation(cx);
            assert_ne!(disabled, initial);
            update(cx, |preferences| {
                preferences.extensions.order.push("fixture-renderer".into())
            });
            let reordered = super::super::extensions::render_generation(cx);
            assert_ne!(reordered, disabled);
            update(cx, |preferences| {
                preferences
                    .extensions
                    .set_badge_place("fixture-renderer", crate::preferences::BadgePlace::Hidden)
            });
            assert_eq!(super::super::extensions::render_generation(cx), reordered);
        });
    }
}
