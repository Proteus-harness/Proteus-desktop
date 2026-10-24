// Адаптировано из Zed: crates/workspace/src/pane.rs (navigation actions).
//! One per-window route history for mouse navigation and keyboard shortcuts.

use gpui_kit::*;

use super::extensions::{self, OpenDocument};
use super::workspace::{Status, Workspace};
use crate::settings::catalog::PageId;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Destination {
    Tab(String),
    Settings { chat: String, page: PageId },
}

impl Workspace {
    pub(super) fn record_navigation(&mut self, cx: &App) {
        if !matches!(self.status, Status::Ready) {
            return;
        }
        let destination = if let Some(settings) = &self.settings {
            let (chat, page) = settings.read(cx).location();
            Destination::Settings { chat, page }
        } else {
            let layout = self.tab_layout(cx);
            let Some(id) = layout.active(layout.focused) else {
                return;
            };
            Destination::Tab(id.to_string())
        };
        self.navigation.visit(destination);
    }

    pub(super) fn navigate(
        &mut self,
        direction: NavigationDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.record_navigation(cx);
        let layout = self.tab_layout(cx);
        let previous = self.navigation.clone();
        let available = |target: &Destination| match target {
            Destination::Tab(id) => layout.contains(id) || extensions::document(id, cx).is_some(),
            Destination::Settings { chat, page } => {
                self.chats.contains_key(chat)
                    && layout.contains(chat)
                    && match page {
                        // Installed but disabled extensions still have a
                        // settings page with their enable switch and details.
                        PageId::Extension(id) => extensions::find(id).is_some(),
                        _ => true,
                    }
            }
        };
        let target = match direction {
            NavigationDirection::Back => self.navigation.back(available),
            NavigationDirection::Forward => self.navigation.forward(available),
        };
        let Some(target) = target else { return };
        let leaves_settings = match &target {
            Destination::Tab(_) => true,
            Destination::Settings { chat, .. } => self
                .settings
                .as_ref()
                .is_some_and(|view| view.read(cx).location().0 != *chat),
        };
        if leaves_settings
            && let Some(settings) = self.settings.clone()
            && settings.read(cx).has_unsaved_changes()
        {
            self.navigation = previous;
            settings.update(cx, |view, cx| view.request_close(cx));
            return;
        }
        // A late resume must not replace the destination the user just chose.
        self.opening.clear();
        match target {
            Destination::Tab(id) => {
                self.close_settings(cx);
                if let Some(pane) = layout.pane_of(&id) {
                    self.activate_tab(pane, &id, window, cx);
                } else {
                    // Preview tabs may have been replaced. Reopen their
                    // public document address through the normal host path.
                    self.open_document(
                        &id,
                        OpenDocument {
                            preview: true,
                            focus: true,
                        },
                        window,
                        cx,
                    );
                }
            }
            Destination::Settings { chat, page } => {
                if self
                    .settings
                    .as_ref()
                    .is_some_and(|view| view.read(cx).location().0 != chat)
                {
                    self.close_settings(cx);
                }
                if self.active_chat.as_ref() != Some(&chat) {
                    if let Some(pane) = layout.pane_of(&chat) {
                        self.activate_tab(pane, &chat, window, cx);
                    }
                }
                self.open_settings(window, cx);
                if let Some(settings) = self.settings.clone() {
                    settings.update(cx, |view, cx| view.navigate_to(page, window, cx));
                }
            }
        }
        cx.notify();
    }
}
