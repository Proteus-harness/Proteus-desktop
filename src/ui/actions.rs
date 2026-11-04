use gpui_kit::*;

use super::settings::{SettingsContext, SettingsEvent, SettingsView};
use super::workspace::Workspace;

impl Workspace {
    /// Client settings are independent of the active chat's agent backend.
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_settings_for(self.active_chat_key(), window, cx);
    }

    pub(super) fn open_settings_for(
        &mut self,
        chat: Option<crate::tabs::ChatKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.invalidate_navigation();
        if self.settings.is_some() {
            return;
        }
        self.creating_chat = None;
        let context = SettingsContext::for_chat(chat, || self.api());
        let workspace = cx.weak_entity();
        let view = cx.new(|cx| SettingsView::new(workspace, context, window, cx));
        self.settings_subscription = Some(cx.subscribe_in(
            &view,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Saved => this.reload_chat_configs(cx),
                SettingsEvent::NavigationChanged => cx.notify(),
                SettingsEvent::Close => {
                    this.close_settings(cx);
                    this.focus_active_chat(window, cx);
                }
            },
        ));
        self.settings = Some(view);
        self.release_hidden_chat_activities(&self.layout, cx);
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings.take().is_some() {
            self.settings_subscription = None;
            cx.notify();
        }
    }

    pub(in crate::ui) fn invalidate_navigation(&mut self) {
        self.navigation_generation = self.navigation_generation.wrapping_add(1);
        self.creating_chat = None;
    }

    /// Every explicit destination uses the Settings guard before it can
    /// replace the screen or start an operation. A declined destination also
    /// invalidates older pending navigation.
    pub(in crate::ui) fn begin_navigation(&mut self, cx: &mut Context<Self>) -> bool {
        self.invalidate_navigation();
        if let Some(settings) = self.settings.clone()
            && !settings.update(cx, |settings, cx| settings.check_close(cx))
        {
            return false;
        }
        self.close_settings(cx);
        true
    }
}
