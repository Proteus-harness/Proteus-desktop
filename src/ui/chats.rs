//! Open chats of the workspace: one [`Chat`] per chat tab, the active chat
//! that commands and extensions address, and the session list around them.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui_kit::*;

use super::chat::{Chat, ChatEvent, ComposerExtras, ComposerSlot};
use super::extensions::{HostEvent, OpenDocument};
use super::format;
use super::workspace::Workspace;
use crate::conversations::AgentKind;
use crate::preferences::BadgePlace;
use crate::tabs::{self, ChatKey, PaneId, TabLayout};
mod codex;
mod pending;

/// A chat view and the subscription to its events.
pub(super) struct OpenChat {
    pub view: Entity<Chat>,
    _events: Subscription,
}

impl Workspace {
    pub(super) fn active_chat_view(&self) -> Option<&Entity<Chat>> {
        self.chats
            .get(self.active_chat.as_ref()?)
            .map(|chat| &chat.view)
    }

    pub(super) fn active_chat_dir(&self) -> Option<&Path> {
        tabs::chat_dir(self.active_chat.as_deref()?)
    }

    pub(super) fn active_chat_key(&self) -> Option<ChatKey> {
        tabs::chat_key(self.active_chat.as_deref()?)
    }

    /// Title of a session: the name the user gave it, else what the session
    /// list knows.
    pub(super) fn session_title(&self, dir: &ChatKey, cx: &App) -> String {
        if let Some(title) = super::prefs::current(cx).sessions.title(dir) {
            return title.to_owned();
        }
        self.sessions
            .iter()
            .find(|summary| &summary.key == dir)
            .map(format::session_title)
            .unwrap_or_else(|| "Новая сессия".to_owned())
    }

    /// Builds the chats of the saved layout and closes chat tabs whose
    /// sessions are gone.
    pub(super) fn restore_chats(
        &mut self,
        dirs: Vec<PathBuf>,
        resumed: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for dir in resumed {
            self.insert_chat(dir, window, cx);
        }
        let chats: Vec<String> = dirs.iter().map(|dir| tabs::chat(dir)).collect();
        self.update_tabs(cx, |layout| {
            layout.retain(|id| tabs::chat_dir(id).is_none() || chats.iter().any(|chat| chat == id))
        });
    }

    /// Opens a resumed or new session as a regular tab with the keyboard in
    /// its composer.
    pub(super) fn add_chat(
        &mut self,
        dir: PathBuf,
        pane: Option<PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.insert_chat(dir, window, cx);
        self.show_tab_in(&id, pane, window, cx);
    }

    /// Shows the chat of a session, resuming the session first when its tab
    /// is not open. A preview chat is replaced by the next preview.
    pub(super) fn open_chat(
        &mut self,
        key: ChatKey,
        open: OpenDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deleting_sessions.contains(&key) {
            return;
        }
        self.creating_chat = None;
        self.close_settings(cx);
        let id = key.tab_id();
        if self.show_external_tab(&id, cx) {
            return;
        }
        if self.chats.contains_key(&id) {
            self.show_tab(&id, open, window, cx);
            return;
        }
        // Selection belongs to the click, never to the order of RPC replies.
        // The shared pane already renders an unloaded chat as a loading page.
        self.show_tab(&id, open, window, cx);
        self.resume_key(key, Some(open), window, cx);
    }

    /// Resume a saved tab without moving it or changing the user's selection.
    pub(super) fn ensure_chat(
        &mut self,
        key: ChatKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resume_key(key, None, window, cx);
    }

    fn resume_key(
        &mut self,
        key: ChatKey,
        open: Option<OpenDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.deleting_sessions.contains(&key) {
            return;
        }
        match key {
            ChatKey::Proteus(dir) => self.resume_chat(dir, open, window, cx),
            ChatKey::Codex(id) => self.resume_codex(id, open, window, cx),
        }
    }

    fn resume_chat(
        &mut self,
        dir: PathBuf,
        open: Option<OpenDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = tabs::chat(&dir);
        // A double click lands while the first click is still resuming.
        if let Some(pending) = self.opening.get(&id) {
            if let Some(open) = open {
                let previous = pending.get().unwrap_or(open);
                pending.set(Some(OpenDocument {
                    preview: previous.preview && open.preview,
                    focus: previous.focus || open.focus,
                }));
            }
            return;
        }
        let pending = Rc::new(Cell::new(open));
        self.opening.insert(id.clone(), pending.clone());
        self.call_in(
            window,
            cx,
            move |api| Ok(api.resume(&dir)),
            move |this, resumed, window, cx| {
                let Some(open) = pending::take(&mut this.opening, &id, &pending, &this.layout)
                else {
                    return;
                };
                let opened = match resumed {
                    Ok(opened) => opened,
                    Err(error) => {
                        this.update_tabs(cx, |tabs| tabs.close(&id));
                        return this.report(format!("{error:#}"), cx);
                    }
                };
                this.session_projects
                    .insert(opened.session_dir.clone(), opened.workspace_path);
                let id = this.insert_chat(opened.session_dir, window, cx);
                this.finish_chat_resume(&id, open, window, cx);
            },
        );
    }

    /// Restores a closed tab immediately, then fills its loading page.
    /// Closing the UI tab never deleted the session.
    pub(super) fn restore_closed_chat(
        &mut self,
        closed: tabs::ClosedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(key) = tabs::chat_key(&closed.id) else {
            return;
        };
        if self.deleting_sessions.contains(&key) {
            return;
        }
        self.update_tabs(cx, |tabs| tabs.restore_closed(&closed));
        if self.chats.contains_key(&closed.id) {
            self.focus_active(window, cx);
        } else {
            self.resume_key(
                key,
                Some(OpenDocument {
                    preview: false,
                    focus: true,
                }),
                window,
                cx,
            );
        }
    }

    /// A delayed load only fills its existing tab. It never reopens a closed
    /// preview, changes selection, or focuses a chat behind Settings.
    fn finish_chat_resume(
        &mut self,
        id: &str,
        open: Option<OpenDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = self.tab_layout(cx);
        self.sync_chats(&layout, cx);
        if pending::may_focus(
            &layout,
            id,
            self.settings.is_some(),
            open.is_none_or(|open| open.focus),
        ) {
            self.focus_active_chat(window, cx);
        }
        cx.notify();
    }

    /// Click on the session list: one click previews the chat, a double
    /// click keeps it and moves the keyboard there.
    pub(super) fn select_session(
        &mut self,
        key: ChatKey,
        clicks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keep = clicks > 1;
        let open = OpenDocument {
            preview: !keep,
            focus: keep,
        };
        self.open_chat(key, open, window, cx);
    }

    pub(super) fn new_session_for(
        &mut self,
        agent: AgentKind,
        pane: Option<PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::prefs::update(cx, |prefs| prefs.preferred_agent = agent);
        self.new_session(pane, window, cx);
    }

    /// Starts a session; its chat opens in `pane` when the user chose one.
    pub(super) fn new_session(
        &mut self,
        pane: Option<PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if super::prefs::current(cx).preferred_agent == AgentKind::Codex {
            return self.new_codex(self.active_project_path().to_path_buf(), pane, window, cx);
        }
        self.close_settings(cx);
        let requested_layout = self.layout.clone();
        let creation = self.begin_chat_creation();
        let source = self.active_chat_dir().map(Path::to_path_buf);
        let project = self.active_project_path().to_path_buf();
        self.call_in(
            window,
            cx,
            move |api| {
                let opened = api.new_session(source.as_deref(), Some(&project))?;
                Ok((opened, api.sessions()?))
            },
            move |this, (opened, sessions), window, cx| {
                this.session_projects
                    .insert(opened.session_dir.clone(), opened.workspace_path);
                this.set_sessions(sessions, cx);
                if this.finish_chat_creation(&creation, &requested_layout) {
                    this.add_chat(opened.session_dir, pane, window, cx);
                }
            },
        );
    }

    fn begin_chat_creation(&mut self) -> Rc<()> {
        let ticket = Rc::new(());
        self.creating_chat = Some(ticket.clone());
        ticket
    }

    fn finish_chat_creation(&mut self, ticket: &Rc<()>, requested: &TabLayout) -> bool {
        pending::take_creation(&mut self.creating_chat, ticket)
            && self.settings.is_none()
            && &self.layout == requested
    }

    /// Builds the chat of a session unless it exists; returns its tab id.
    fn insert_chat(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> String {
        let id = tabs::chat(&dir);
        if self.chats.contains_key(&id) {
            return id;
        }
        let Some(api) = self.api() else {
            return id;
        };
        let extras = self.composer_extras(cx);
        let workspace = cx.weak_entity();
        let stream = self.startup_streams.remove(&dir);
        let view = cx.new(|cx| Chat::new(api, workspace, dir, stream, extras, window, cx));
        self.bind_chat(&id, view, cx);
        id
    }

    /// Rebind an existing chat to its destination window without replacing
    /// its session, pending run, composer or rendered transcript caches.
    pub(super) fn adopt_chat(
        &mut self,
        id: &str,
        view: Entity<Chat>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.weak_entity();
        let extras = self.composer_extras(cx);
        view.update(cx, |chat, cx| chat.rehost(workspace, extras, window, cx));
        self.bind_chat(id, view, cx);
    }

    pub(super) fn bind_chat(&mut self, id: &str, view: Entity<Chat>, cx: &mut Context<Self>) {
        let tab = id.to_owned();
        let events = cx.subscribe(&view, move |this, chat, event: &ChatEvent, cx| {
            this.on_chat_event(&tab, &chat, event, cx)
        });
        self.chats.insert(
            id.to_owned(),
            OpenChat {
                view,
                _events: events,
            },
        );
    }

    /// Both public extension places share the activity strip above the
    /// input. Retained badges belong to the active chat.
    pub(super) fn composer_extras(&self, cx: &Context<Self>) -> ComposerExtras {
        let workspace = cx.weak_entity();
        Rc::new(move |chat, slot, cx| {
            let workspace = workspace.upgrade()?.read(cx);
            let place = match slot {
                ComposerSlot::Start => BadgePlace::ComposerStart,
                ComposerSlot::End => BadgePlace::ComposerEnd,
            };
            (workspace.active_chat_view()?.entity_id() == chat)
                .then(|| workspace.render_badge_strip(place, cx))
                .flatten()
        })
    }

    fn on_chat_event(
        &mut self,
        tab: &str,
        chat: &Entity<Chat>,
        event: &ChatEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            ChatEvent::SessionStateChanged | ChatEvent::SessionEntriesChanged(_) => {
                if self.active_chat_view() != Some(chat) {
                    return;
                }
                if !chat.read(cx).supports_core() {
                    return;
                }
                let session = &chat.read(cx).session;
                let update = match event {
                    ChatEvent::SessionEntriesChanged(change) => {
                        super::extensions::session::changed(session, *change)
                    }
                    _ => Some(crate::declarative::session::Update::State {
                        state: Box::new(super::extensions::session::state(
                            session,
                            self.sessions
                                .iter()
                                .find(|summary| summary.key == session.key)
                                .and_then(|summary| summary.activity.clone()),
                        )),
                    }),
                };
                if let Some(update) = update {
                    cx.emit(super::extensions::session::SessionChanged {
                        session_dir: session.dir.clone(),
                        update,
                    });
                }
                return;
            }
            ChatEvent::RunSettled => {
                self.refresh_sessions(cx);
                if matches!(tabs::chat_key(tab), Some(ChatKey::Codex(_))) {
                    self.refresh_codex_sessions(cx);
                }
                if self.active_chat_view() == Some(chat) {
                    cx.emit(HostEvent::RunSettled);
                }
            }
            ChatEvent::CatalogUpdated => {
                if self.active_chat.as_deref() == Some(tab) {
                    self.refresh_sessions(cx);
                }
            }
            ChatEvent::Activity {
                session_dir,
                activity,
            } => {
                // Repeated copies from other streams carry no change.
                self.attend(session_dir, activity.status, cx);
                // Every session stream carries every update; only the active
                // chat's copy counts, and a switch reloads the list anyway.
                if self.active_chat.as_deref() != Some(tab) {
                    return;
                }
                match self
                    .sessions
                    .iter_mut()
                    .find(|summary| summary.key.proteus_dir() == Some(session_dir.as_path()))
                {
                    Some(summary) => summary.activity = Some(activity.clone()),
                    None => self.refresh_sessions(cx),
                }
            }
            ChatEvent::Sent => self.keep_tab(tab, cx),
            ChatEvent::ServerStopped { connection } => {
                let group = self.windows.clone();
                let connection = *connection;
                cx.defer(move |cx| group.server_stopped(connection, "Proteus Core остановлен", cx));
            }
        }
        cx.notify();
    }

    /// Reloads the session list; requests made while one runs coalesce.
    /// Replace catalog metadata without pretending a session opened again.
    /// Existing scopes compare their resolved folder before sending anything.
    pub(super) fn set_sessions(
        &mut self,
        sessions: Vec<proteus_contracts::app_protocol::AppSessionSummary>,
        cx: &mut Context<Self>,
    ) {
        self.session_projects
            .retain(|dir, _| !sessions.iter().any(|summary| summary.session_dir == *dir));
        self.sessions
            .retain(|summary| matches!(summary.key, ChatKey::Codex(_)));
        self.sessions
            .extend(crate::conversations::from_proteus(sessions));
        cx.emit(HostEvent::ContextChanged);
    }

    pub(super) fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        // Metadata refresh must never create a Core for a Codex-only window.
        if self.api().is_none() {
            return;
        }
        if self.sessions_loading {
            self.sessions_stale = true;
            return;
        }
        self.sessions_loading = true;
        let epoch = self.session_catalog_epoch;
        self.call(
            cx,
            |api| Ok(api.sessions()),
            move |this, sessions, cx| {
                this.sessions_loading = false;
                if epoch != this.session_catalog_epoch {
                    this.sessions_stale = false;
                    this.refresh_sessions(cx);
                    return;
                }
                match sessions {
                    Ok(sessions) => {
                        for summary in &sessions {
                            if let Some(activity) = &summary.activity {
                                this.attention
                                    .baseline(&summary.session_dir, activity.status);
                            }
                        }
                        let known: std::collections::HashSet<_> = sessions
                            .iter()
                            .map(|session| tabs::chat(&session.session_dir))
                            .collect();
                        let open: std::collections::HashSet<_> =
                            this.chats.keys().cloned().collect();
                        this.update_tabs(cx, |layout| {
                            layout.retain(|id| {
                                tabs::chat_dir(id).is_none()
                                    || known.contains(id)
                                    || open.contains(id)
                            });
                        });
                        this.sessions_arrived
                            .get_or_insert_with(std::time::Instant::now);
                        this.set_sessions(sessions, cx);
                    }
                    Err(error) => this.report(format!("{error:#}"), cx),
                }
                if std::mem::take(&mut this.sessions_stale) {
                    this.refresh_sessions(cx);
                }
            },
        );
    }

    /// Drops chats whose tabs closed and follows the layout to the active
    /// chat; extensions reread the session when it changes.
    pub(super) fn sync_chats(&mut self, layout: &TabLayout, cx: &mut Context<Self>) {
        self.chats.retain(|id, _| layout.contains(id));
        self.release_hidden_chat_activities(layout, cx);
        // A load owns its subscription until its acknowledgement arrives.
        // A quick reopen must reuse it instead of registering the same thread
        // twice while the first result is still queued for the UI.
        let active = layout.active_chat(self.active_chat.as_deref());
        if active != self.active_chat {
            // Activity updates are taken from the active chat's stream; a
            // reload covers what arrived elsewhere meanwhile.
            if active.is_some() {
                self.notice = None;
                self.refresh_sessions(cx);
            }
            // Chats draw from their own cache; the composer icons move to
            // the new active one.
            for id in [&self.active_chat, &active].into_iter().flatten() {
                if let Some(chat) = self.chats.get(id) {
                    chat.view.update(cx, |_, cx| cx.notify());
                }
            }
            self.active_chat = active;
            cx.emit(HostEvent::SessionOpened);
            cx.notify();
        }
    }

    pub(super) fn focus_active_chat(&self, window: &mut Window, cx: &mut App) {
        if let Some(chat) = self.active_chat_view() {
            chat.update(cx, |chat, cx| chat.focus(window, cx));
        }
    }

    /// Shows a failed call in the active chat, or in the empty area.
    pub(super) fn report(&mut self, message: String, cx: &mut Context<Self>) {
        match self.active_chat_view() {
            Some(chat) => chat.update(cx, |chat, cx| chat.show_notice(message, cx)),
            None => self.notice = Some(message.into()),
        }
        cx.notify();
    }

    /// Profile settings changed; every chat rereads its session config.
    pub(super) fn reload_chat_configs(&self, cx: &mut Context<Self>) {
        for chat in self.chats.values() {
            chat.view.update(cx, |chat, cx| chat.load_config(cx));
        }
    }
}
