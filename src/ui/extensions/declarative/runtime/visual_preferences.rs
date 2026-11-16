//! Package visual settings belong to an addressed window, independently of
//! the agent session and of other contexts retained by the same process.

use super::*;
use crate::declarative::protocol::WindowVisual;
use crate::ui::prefs::{self, WindowVisualPreferences};

impl Runtime {
    pub(super) fn visual_snapshot(&self, window: AnyWindowHandle, cx: &App) -> WindowVisual {
        let Some(visual) = cx.window_global::<WindowVisualPreferences>(window.window_id()) else {
            return WindowVisual::default();
        };
        WindowVisual {
            revision: visual.revision,
            preferences: visual
                .value
                .package_visual_settings
                .get(self.extension_id())
                .cloned()
                .unwrap_or_default(),
        }
    }

    fn enabled_in(&self, window: AnyWindowHandle, cx: &App) -> bool {
        cx.window_global::<WindowVisualPreferences>(window.window_id())
            .is_some_and(|visual| visual.value.extensions.is_enabled(self.extension_id()))
    }

    pub(super) fn allowed_in_any_window(&self, cx: &App) -> bool {
        self.lifetime.allowed(
            self.scopes
                .values()
                .any(|scope| self.enabled_in(scope.window, cx)),
        )
    }

    pub(super) fn visual_preferences_changed(&mut self, cx: &mut Context<Self>) {
        if !self.allowed_in_any_window(cx) {
            if self.process.is_some() {
                self.stop(Reset::Stop, cx);
            }
            return;
        }
        if self.process.is_none() {
            if self.error.is_none() {
                self.start(cx);
            }
            return;
        }
        let changed: Vec<_> = self
            .scopes
            .iter()
            .filter_map(|(context, scope)| {
                let snapshot = self.visual_snapshot(scope.window, cx);
                (scope.visual_revision != Some(snapshot.revision))
                    .then(|| (context.clone(), snapshot, self.enabled_in(scope.window, cx)))
            })
            .collect();
        for (context, window_visual, enabled) in changed {
            let Some(scope) = self.scopes.get_mut(&context) else {
                continue;
            };
            scope.visual_revision = Some(window_visual.revision);
            let message = Outgoing::Context {
                context: context.clone(),
                workspace: scope.workspace.clone(),
                session_dir: scope.session.clone(),
                window_visual,
            };
            self.send(&message);
            if !self.lifetime.allowed(enabled) {
                continue;
            }
            let views: Vec<_> = self
                .views
                .iter()
                .filter(|(_, view)| view.context == context)
                .map(|(id, _)| id.clone())
                .collect();
            for view in views {
                self.render(&view, cx);
            }
        }
    }

    pub(super) fn set_visual_preferences(
        &mut self,
        context: String,
        view: String,
        id: String,
        expected_revision: u64,
        values: serde_json::Map<String, serde_json::Value>,
        cx: &mut Context<Self>,
    ) {
        let Some(scope) = self.scopes.get(&context) else {
            return;
        };
        let window = scope.window;
        let package = self.extension_id();
        let error = if self.views.get(&view).is_none_or(|view| {
            view.context != context || view.window.window_id() != window.window_id()
        }) {
            Some("visual preferences target a closed or moved view".to_owned())
        } else if id.is_empty() || id.len() > 256 {
            Some("invalid visual preference request id".to_owned())
        } else if crate::visual_config::validate_package_preferences(&values).is_err() {
            Some("visual preferences exceed 64 KiB".to_owned())
        } else {
            match window.update(cx, |_, _, cx| {
                prefs::update_package_visual(package, expected_revision, values, cx)
            }) {
                Ok(Ok(_)) => None,
                Ok(Err(error)) => Some(error),
                Err(error) => Some(format!("visual window unavailable: {error:#}")),
            }
        };
        let snapshot = self.visual_snapshot(window, cx);
        self.send(&Outgoing::VisualPreferencesResult {
            context,
            id,
            revision: snapshot.revision,
            values: snapshot.preferences,
            error,
        });
    }
}
