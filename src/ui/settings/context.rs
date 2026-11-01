// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs
// (open_settings_editor: user settings do not require a project target).
//! Client settings are available for every agent. A Proteus profile is an
//! optional, explicitly owned target, never borrowed from another open chat.

use std::path::PathBuf;

use crate::{api::Api, settings::catalog::PageId, tabs::ChatKey};

#[derive(Clone)]
pub(super) struct Profile {
    pub api: Api,
    pub dir: PathBuf,
}

pub(in crate::ui) struct SettingsContext {
    pub(super) chat: Option<String>,
    pub(super) profile: Option<Profile>,
}

impl SettingsContext {
    pub(in crate::ui) fn for_chat(
        chat: Option<ChatKey>,
        api: impl FnOnce() -> Option<Api>,
    ) -> Self {
        let profile = match &chat {
            Some(ChatKey::Proteus(dir)) => api().map(|api| Profile {
                api,
                dir: dir.clone(),
            }),
            Some(ChatKey::Codex(_)) | None => None,
        };
        Self {
            chat: chat.map(|chat| chat.tab_id()),
            profile,
        }
    }

    pub(super) fn initial_page(&self) -> PageId {
        if self.profile.is_some() {
            PageId::Model
        } else {
            PageId::Appearance
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_settings_keep_native_identity_and_never_request_core() {
        let context =
            SettingsContext::for_chat(Some(ChatKey::Codex("opaque/thread".into())), || {
                panic!("Codex client settings must not request a Proteus connection")
            });
        assert_eq!(
            context.chat,
            Some(ChatKey::Codex("opaque/thread".into()).tab_id())
        );
        assert!(context.profile.is_none());
        assert_eq!(context.initial_page(), PageId::Appearance);
    }

    #[test]
    fn proteus_settings_keep_the_exact_profile_and_connection() {
        let api = Api::new("http://127.0.0.1:1", "synthetic");
        let connection = api.connection_id();
        let dir = PathBuf::from("/synthetic/session");
        let chat = ChatKey::Proteus(dir.clone());
        let context = SettingsContext::for_chat(Some(chat.clone()), || Some(api));
        assert_eq!(context.chat, Some(chat.tab_id()));
        assert_eq!(context.initial_page(), PageId::Model);
        let profile = context.profile.unwrap();
        assert_eq!(profile.dir, dir);
        assert_eq!(profile.api.connection_id(), connection);
    }

    #[test]
    fn disconnected_proteus_still_has_client_settings() {
        let context = SettingsContext::for_chat(
            Some(ChatKey::Proteus(PathBuf::from("/synthetic/session"))),
            || None,
        );
        assert!(context.profile.is_none());
        assert_eq!(context.initial_page(), PageId::Appearance);
    }

    #[test]
    fn no_chat_still_opens_global_settings_without_core_or_fake_identity() {
        let context = SettingsContext::for_chat(None, || {
            panic!("Global settings must not request a Proteus connection")
        });
        assert!(context.chat.is_none());
        assert!(context.profile.is_none());
        assert_eq!(context.initial_page(), PageId::Appearance);
    }
}
