//! Codex lifecycle uses native opaque thread IDs; the Chat view is shared.
use super::*;
use crate::agents::codex::Opened;

impl Workspace {
    pub(in crate::ui) fn refresh_codex_sessions(&mut self, cx: &mut Context<Self>) {
        if self.codex_catalog_loading {
            return;
        }
        self.codex_catalog_loading = true;
        let epoch = self.session_catalog_epoch;
        let slot = self.windows.codex_slot();
        let project = self.active_project_path().to_path_buf();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let service = slot.lock().unwrap().clone();
                    service
                        .map(|service| service.list(&project, None))
                        .transpose()
                })
                .await;
            this.update(cx, |this, cx| {
                this.codex_catalog_loading = false;
                if epoch != this.session_catalog_epoch {
                    this.refresh_codex_sessions(cx);
                    return;
                }
                match result {
                    Ok(Some(page)) => {
                        for row in page.data {
                            let row = crate::conversations::from_codex(&row);
                            if this.deleting_sessions.contains(&row.key) {
                                continue;
                            }
                            crate::conversations::upsert_summary(&mut this.sessions, row);
                        }
                        this.sessions_arrived
                            .get_or_insert_with(std::time::Instant::now);
                    }
                    Ok(None) => {}
                    Err(error) => this.report(format!("Codex: {error:#}"), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(in crate::ui) fn insert_codex_chat(
        &mut self,
        opened: Opened,
        is_new: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        let summary = crate::conversations::from_codex(&opened.metadata);
        let id = summary.key.tab_id();
        crate::conversations::upsert_summary(&mut self.sessions, summary);
        if self.chats.contains_key(&id) {
            return id;
        }
        let extras = self.composer_extras(cx);
        let workspace = cx.weak_entity();
        let models = self.codex_models.clone();
        let view =
            cx.new(|cx| Chat::new_codex(opened, models, is_new, workspace, extras, window, cx));
        self.bind_chat(&id, view, cx);
        id
    }

    pub(in crate::ui) fn new_codex(
        &mut self,
        project: PathBuf,
        pane: Option<PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_settings(cx);
        let requested_layout = self.layout.clone();
        let creation = self.begin_chat_creation();
        self.call_codex_in(
            window,
            cx,
            move |service| {
                let models = service.models()?;
                Ok((service.new_thread(&project)?, models))
            },
            move |this, (opened, models), window, cx| {
                this.codex_models = models;
                if this.finish_chat_creation(&creation, &requested_layout) {
                    let key = ChatKey::Codex(opened.handle.thread_id().to_owned());
                    this.select_chat_space(&key, cx);
                    let id = this.insert_codex_chat(opened, true, window, cx);
                    let pane = pane.filter(|pane| this.layout.pane(*pane).is_some());
                    this.show_tab_in(&id, pane, window, cx);
                } else {
                    // Creation still succeeds, but a newer destination wins.
                    let summary = crate::conversations::from_codex(&opened.metadata);
                    crate::conversations::upsert_summary(&mut this.sessions, summary);
                }
                this.refresh_codex_sessions(cx);
            },
        );
    }

    pub(super) fn resume_codex(
        &mut self,
        thread: String,
        open: Option<OpenDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = tabs::codex(&thread);
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
        self.call_codex_in(
            window,
            cx,
            move |service| {
                Ok(service
                    .resume(&thread)
                    .and_then(|opened| Ok((opened, service.models()?))))
            },
            move |this, result, window, cx| {
                let Some(open) = pending::take(&mut this.opening, &id, &pending, &this.spaces)
                else {
                    return;
                };
                let (opened, models) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        return this.fail_chat_resume(&id, format!("Codex: {error:#}"), cx);
                    }
                };
                this.codex_models = models;
                let id = this.insert_codex_chat(opened, false, window, cx);
                this.finish_chat_resume(&id, open, window, cx);
            },
        );
    }

    pub(in crate::ui) fn delete_session(
        &mut self,
        key: ChatKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.deleting_sessions.insert(key.clone()) {
            return;
        }
        self.session_catalog_epoch = self.session_catalog_epoch.wrapping_add(1);
        match key.clone() {
            ChatKey::Proteus(dir) => self.call_in(
                window,
                cx,
                move |api| Ok(api.delete_session(&dir).map(|_| ())),
                move |this, result, window, cx| this.finish_delete(key, result, window, cx),
            ),
            ChatKey::Codex(id) => self.call_codex_in(
                window,
                cx,
                move |service| Ok(service.delete_thread(&id).map_err(Into::into)),
                move |this, result, window, cx| this.finish_delete(key, result, window, cx),
            ),
        }
        cx.notify();
    }

    fn finish_delete(
        &mut self,
        key: ChatKey,
        result: anyhow::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.deleting_sessions.remove(&key);
        self.session_catalog_epoch = self.session_catalog_epoch.wrapping_add(1);
        match result {
            Ok(()) => self.forget_chat(key.clone(), window, cx),
            Err(error) => self.report(format!("Не удалось удалить чат: {error:#}"), cx),
        }
        match key {
            ChatKey::Codex(_) => self.refresh_codex_sessions(cx),
            ChatKey::Proteus(_) => self.refresh_sessions(cx),
        }
    }

    fn forget_chat(&mut self, key: ChatKey, window: &mut Window, cx: &mut Context<Self>) {
        super::super::prefs::update(cx, |prefs| prefs.sessions.forget(&key));
        let core = matches!(key, ChatKey::Proteus(_));
        let keys = [key].into_iter().collect();
        self.forget_chats(&keys, window, cx);
        if core {
            self.windows.forget_elsewhere(cx.entity_id(), &keys, cx);
        }
    }

    pub(in crate::ui) fn forget_chats(
        &mut self,
        keys: &std::collections::HashSet<ChatKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.session_catalog_epoch = self.session_catalog_epoch.wrapping_add(1);
        self.deleting_sessions.retain(|key| !keys.contains(key));
        let layout = self.tab_layout(cx);
        let was_active = self
            .spaces
            .active_root()
            .and_then(tabs::chat_key)
            .is_some_and(|key| keys.contains(&key))
            || layout
                .active(layout.focused)
                .and_then(tabs::chat_key)
                .is_some_and(|key| keys.contains(&key));
        self.sessions.retain(|summary| !keys.contains(&summary.key));
        self.opening
            .retain(|id, _| tabs::chat_key(id).is_none_or(|key| !keys.contains(&key)));
        let ids = keys.iter().map(ChatKey::tab_id).collect();
        self.remove_space_chats(&ids, cx);
        if was_active && self.settings.is_none() {
            self.focus_active(window, cx);
        }
        cx.notify();
    }
}
