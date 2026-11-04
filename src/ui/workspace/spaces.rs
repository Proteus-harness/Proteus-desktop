//! Адаптировано из Zed: crates/workspace/src/multi_workspace.rs (activate).
//! A root chat owns one retained pane tree inside the native window. Switching
//! trees moves handles, never clones a transcript, PTY, or extension process.

use std::collections::{HashMap, HashSet};

use gpui_kit::*;

use super::Workspace;
use crate::conversations::ConversationSummary;
use crate::tabs::{self, ChatKey};
use crate::ui::extensions::{HostEvent, SurfaceView};

#[derive(Default)]
pub(super) struct SpaceViews {
    pub(super) documents: HashMap<String, SurfaceView>,
    pub(super) titles: HashMap<String, SharedString>,
    pub(super) navigation: crate::navigation::History<crate::ui::navigation::Destination>,
    pub(super) strips: HashMap<tabs::PaneId, (ScrollHandle, Option<String>)>,
    notice: Option<SharedString>,
}

impl Workspace {
    pub(in crate::ui) fn retained_documents(
        &self,
    ) -> impl Iterator<Item = (&String, &SurfaceView)> {
        self.documents.iter().chain(
            self.parked_spaces
                .values()
                .flat_map(|space| space.documents.iter()),
        )
    }

    /// In-flight read results may fill the same retained hidden document, but
    /// cannot fill a closed tab or its replacement after a quick reopen.
    pub(in crate::ui) fn document_current(&self, id: &str, entity: EntityId) -> bool {
        self.documents
            .get(id)
            .is_some_and(|surface| surface.view.entity_id() == entity)
            || self.parked_spaces.values().any(|space| {
                space
                    .documents
                    .get(id)
                    .is_some_and(|surface| surface.view.entity_id() == entity)
            })
    }

    pub(in crate::ui) fn space_root_key(&self) -> Option<ChatKey> {
        self.spaces
            .active_root()
            .and_then(tabs::chat_key)
            .or_else(|| self.active_chat_key())
    }

    pub(in crate::ui) fn space_root_chat_view(&self) -> Option<&Entity<crate::ui::chat::Chat>> {
        match self.spaces.active_root() {
            Some(root) => self.chats.get(root).map(|chat| &chat.view),
            None => self.active_chat_view(),
        }
    }

    pub(in crate::ui) fn set_document_title_for_space(
        &mut self,
        root: &ChatKey,
        id: &str,
        title: String,
        cx: &mut Context<Self>,
    ) {
        let root = root.tab_id();
        if self.spaces.active_root() == Some(root.as_str()) {
            if self
                .document_titles
                .get(id)
                .is_none_or(|old| old.as_ref() != title)
            {
                self.document_titles.insert(id.to_owned(), title.into());
                cx.notify();
            }
        } else if let Some(parked) = self.parked_spaces.get_mut(&Some(root)) {
            parked.titles.insert(id.to_owned(), title.into());
        }
    }

    fn take_space_views(&mut self) -> SpaceViews {
        SpaceViews {
            documents: std::mem::take(&mut self.documents),
            titles: std::mem::take(&mut self.document_titles),
            navigation: std::mem::take(&mut self.navigation),
            strips: std::mem::take(&mut self.tab_strips),
            notice: self.notice.take(),
        }
    }

    fn restore_space_views(&mut self, root: Option<String>) {
        let stored = self.parked_spaces.remove(&root).unwrap_or_default();
        self.documents = stored.documents;
        self.document_titles = stored.titles;
        self.navigation = stored.navigation;
        self.tab_strips = stored.strips;
        self.notice = stored.notice;
        self.split_drag = None;
        self.tab_drag = false;
        self.tab_drop = None;
        self.tab_motion = Default::default();
    }

    /// Only an explicit navigation changes areas; late metadata never does.
    pub(in crate::ui) fn chat_space_root(&self, key: &ChatKey) -> String {
        resolve_space_root(&self.sessions, &self.spaces, key)
    }

    pub(in crate::ui) fn select_chat_space(
        &mut self,
        key: &ChatKey,
        cx: &mut Context<Self>,
    ) -> bool {
        let root = self.chat_space_root(key);
        let switched = self.select_space_root(&root, cx);
        // A legacy window could contain several unrelated main chats. Move
        // only this tab out of its former tree; never duplicate its entity.
        self.spaces.extract_chat_except(&key.tab_id(), &root);
        self.layout = self.spaces.active_layout().clone();
        self.windows.persist(cx.entity_id(), &self.spaces, cx);
        switched
    }

    pub(in crate::ui) fn select_space_root(&mut self, root: &str, cx: &mut Context<Self>) -> bool {
        if self.spaces.active_root() == Some(root) {
            return false;
        }
        self.creating_chat = None;
        self.spaces.save_active(self.layout.clone());
        let previous = self.spaces.active_root().map(str::to_owned);
        let views = self.take_space_views();
        self.parked_spaces.insert(previous, views);
        self.spaces.select(root);
        self.layout = self.spaces.active_layout().clone();
        self.restore_space_views(Some(root.to_owned()));
        let notice = self.notice.take();
        let layout = self.layout.clone();
        self.sync_chats(&layout, cx);
        self.notice = notice;
        self.windows.persist(cx.entity_id(), &self.spaces, cx);
        cx.emit(HostEvent::SessionOpened);
        cx.emit(HostEvent::NavigationChanged);
        cx.notify();
        true
    }

    /// A failed background resume closes its own loading tab and records its
    /// notice without touching the other displayed area.
    pub(in crate::ui) fn fail_chat_resume(
        &mut self,
        id: &str,
        message: String,
        cx: &mut Context<Self>,
    ) {
        let owner = self.spaces.owner_of_chat(id).map(str::to_owned);
        self.spaces.extract_chat(id);
        self.layout = self.spaces.active_layout().clone();
        let layout = self.layout.clone();
        self.sync_chats(&layout, cx);
        if owner.as_deref() == self.spaces.active_root() {
            self.notice = Some(message.into());
        } else if let Some(parked) = self.parked_spaces.get_mut(&owner) {
            parked.notice = Some(message.into());
        }
        self.windows.persist(cx.entity_id(), &self.spaces, cx);
        cx.notify();
    }

    pub(in crate::ui) fn remove_space_chats(
        &mut self,
        ids: &HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        self.spaces.save_active(self.layout.clone());
        let before = self.spaces.active_root().map(str::to_owned);
        self.spaces.remove_chats(ids.iter().map(String::as_str));
        let keep_document = |id: &str| {
            crate::ui::chat::readonly::Target::parse(id)
                .is_none_or(|target| !ids.contains(&tabs::chat(&target.session.session_dir)))
        };
        self.spaces.retain_documents(keep_document);
        self.documents.retain(|id, _| keep_document(id));
        for parked in self.parked_spaces.values_mut() {
            parked.documents.retain(|id, _| keep_document(id));
            parked.titles.retain(|id, _| keep_document(id));
        }
        self.document_titles.retain(|id, _| keep_document(id));
        let after = self.spaces.active_root().map(str::to_owned);
        if before != after {
            // Explicit deletion closes the area's resources; hiding never does.
            let _ = self.take_space_views();
            self.restore_space_views(after);
        }
        self.parked_spaces
            .retain(|root, _| root.as_ref().is_none_or(|root| !ids.contains(root)));
        self.layout = self.spaces.active_layout().clone();
        let layout = self.layout.clone();
        self.sync_chats(&layout, cx);
        self.windows.persist(cx.entity_id(), &self.spaces, cx);
        cx.emit(HostEvent::SessionOpened);
        cx.emit(HostEvent::NavigationChanged);
        cx.notify();
    }
}

fn resolve_space_root(
    summaries: &[ConversationSummary],
    spaces: &tabs::ChatSpaces,
    key: &ChatKey,
) -> String {
    let id = key.tab_id();
    chat_root(summaries, key)
        .map(|root| root.tab_id())
        .or_else(|| spaces.owner_of_chat(&id).map(str::to_owned))
        .unwrap_or(id)
}

/// Use raw server ancestry, independent of sidebar pin/archive grouping.
/// Cycles have a stable fallback and never spin while drawing the sidebar.
fn chat_root(summaries: &[ConversationSummary], key: &ChatKey) -> Option<ChatKey> {
    let mut current = summaries.iter().find(|row| &row.key == key)?;
    let mut seen = HashSet::new();
    while let Some(parent) = &current.parent {
        if !seen.insert(current.key.clone()) {
            return Some(key.clone());
        }
        let Some(row) = summaries.iter().find(|row| &row.key == parent) else {
            return Some(parent.clone());
        };
        current = row;
    }
    Some(current.key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, parent: Option<&str>) -> ConversationSummary {
        ConversationSummary {
            key: ChatKey::Codex(id.into()),
            proteus_session_id: None,
            parent: parent.map(|id| ChatKey::Codex(id.into())),
            workspace_path: "/project".into(),
            message_count: 0,
            updated_at_ms: None,
            preview: None,
            activity: None,
        }
    }

    #[::core::prelude::v1::test]
    fn subagent_space_follows_raw_ancestors_and_unknown_parent() {
        let rows = [
            row("root", None),
            row("child", Some("root")),
            row("nested", Some("child")),
        ];
        assert_eq!(
            chat_root(&rows, &ChatKey::Codex("nested".into())),
            Some(ChatKey::Codex("root".into()))
        );
        assert_eq!(
            chat_root(
                &[row("child", Some("outside"))],
                &ChatKey::Codex("child".into())
            ),
            Some(ChatKey::Codex("outside".into()))
        );
        assert_eq!(chat_root(&rows, &ChatKey::Codex("unlisted".into())), None);
    }

    #[::core::prelude::v1::test]
    fn corrupt_parent_cycle_is_bounded() {
        let rows = [row("a", Some("b")), row("b", Some("a"))];
        assert_eq!(
            chat_root(&rows, &ChatKey::Codex("a".into())),
            Some(ChatKey::Codex("a".into()))
        );
    }

    #[::core::prelude::v1::test]
    fn old_main_tab_chooses_its_own_area_while_child_keeps_parent() {
        let mut old = tabs::TabLayout::default();
        old.open("codex:a", |_| false);
        old.open("codex:b", |_| false);
        old.open("codex:child", |_| false);
        old.activate(old.pane_of("codex:a").unwrap(), "codex:a");
        let spaces = tabs::ChatSpaces::from_layout(old);
        let rows = [row("a", None), row("b", None), row("child", Some("a"))];
        assert_eq!(spaces.owner_of_chat("codex:b"), Some("codex:a"));
        assert_eq!(
            resolve_space_root(&rows, &spaces, &ChatKey::Codex("b".into())),
            "codex:b"
        );
        assert_eq!(
            resolve_space_root(&rows, &spaces, &ChatKey::Codex("child".into())),
            "codex:a"
        );
        assert_eq!(
            resolve_space_root(&[], &spaces, &ChatKey::Codex("child".into())),
            "codex:a"
        );
    }
}
