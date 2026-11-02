//! Client preferences in the running app: a global plus the effects of each
//! setting (theme, motion level).

mod persistence;

use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::preferences::{self, MotionPreference, Preferences, ThemePreference};

const BASE_FONT: f32 = 16.;
const BASE_MONO_FONT: f32 = 13.;
/// Interface scale: 1.0 at 100%.
pub fn scale(cx: &App) -> f32 {
    current(cx).view.scale_factor()
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
    scaled(f32::from(current(cx).view.chat_text))
}

/// Conversation prose and composer family; code retains the theme's mono.
pub fn chat_font(cx: &App) -> SharedString {
    cx.global::<ClientPreferences>()
        .chat_font
        .clone()
        .unwrap_or_else(|| Theme::global(cx).font_family.clone())
}

/// Widest conversation column.
pub fn chat_width(cx: &App) -> Rems {
    scaled(f32::from(current(cx).view.chat_width))
}

pub struct ClientPreferences {
    pub value: Arc<Preferences>,
    writer: Option<persistence::Writer<Preferences>>,
    revision: u64,
    /// Why the stored file was not used; shown on the client settings pages.
    pub problem: Option<String>,
    chat_font: Option<SharedString>,
}

impl Global for ClientPreferences {}

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
    let chat_font = super::theme::resolve(value.fonts.chat.as_deref(), cx);
    cx.set_global(ClientPreferences {
        value: Arc::new(value),
        problem,
        chat_font,
        writer,
        revision: 0,
    });
    let value = cx.global::<ClientPreferences>().value.clone();
    apply(&value, Effects::initial(), cx);
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

/// Applies the change right away and stores it on this device.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    let previous = &cx.global::<ClientPreferences>().value;
    let mut value = (**previous).clone();
    change(&mut value);
    if value == **previous {
        return;
    }
    let effects = Effects::between(previous, &value);
    let chat_font = if effects.chat_font {
        super::theme::resolve(value.fonts.chat.as_deref(), cx)
    } else {
        cx.global::<ClientPreferences>().chat_font.clone()
    };
    let value = Arc::new(value);
    let submitted = cx
        .global::<ClientPreferences>()
        .writer
        .as_ref()
        .map(|writer| writer.submit(value.clone()));
    let global = cx.global_mut::<ClientPreferences>();
    global.value = value.clone();
    global.chat_font = chat_font;
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
                || previous.sidebar != value.sidebar
                || previous.send != value.send
                || previous.notifications != value.notifications,
        }
    }
}

fn apply(value: &Preferences, effects: Effects, cx: &mut App) {
    if effects.theme {
        super::theme::select(&value.themes, value.fonts.interface.as_deref(), cx);
        match value.theme {
            ThemePreference::System => Theme::sync_system_appearance(None, cx),
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
        super::motion::set_level(value.motion);
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
        after.tabs.open("codex:thread-fixture", |_| false);
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
}
