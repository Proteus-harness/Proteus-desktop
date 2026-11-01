//! Codex lifecycle uses native opaque thread IDs; the Chat view is shared.
use super::*;
use crate::agents::codex::Opened;

impl Workspace {
    pub(in crate::ui) fn refresh_codex_sessions(&mut self, cx: &mut Context<Self>) {
        if self.codex_catalog_loading {
            return;
        }
        self.codex_catalog_loading = true;
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
                match result {
                    Ok(Some(page)) => {
                        for row in page.data {
                            let row = crate::conversations::from_codex(&row);
                            this.sessions.retain(|summary| summary.key != row.key);
                            this.sessions.push(row);
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
        self.sessions.retain(|row| row.key != summary.key);
        self.sessions.push(summary);
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
        self.call_codex_in(
            window,
            cx,
            move |service| {
                let models = service.models()?;
                Ok((service.new_thread(&project)?, models))
            },
            move |this, (opened, models), window, cx| {
                this.codex_models = models;
                let id = this.insert_codex_chat(opened, true, window, cx);
                this.show_tab_in(&id, pane, window, cx);
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
                if !this
                    .opening
                    .get(&id)
                    .is_some_and(|active| Rc::ptr_eq(active, &pending))
                {
                    return;
                }
                this.opening.remove(&id);
                let (opened, models) = match result {
                    Ok(result) => result,
                    Err(error) => {
                        if pending.get().is_none() {
                            this.update_tabs(cx, |tabs| tabs.close(&id));
                        }
                        return this.report(format!("Codex: {error:#}"), cx);
                    }
                };
                this.codex_models = models;
                let id = this.insert_codex_chat(opened, false, window, cx);
                if let Some(open) = pending.get() {
                    this.show_tab(&id, open, window, cx);
                } else {
                    let layout = this.tab_layout(cx);
                    this.sync_chats(&layout, cx);
                    if this.settings.is_none() && layout.active(layout.focused) == Some(&id) {
                        this.focus_active_chat(window, cx);
                    }
                }
            },
        );
    }

    pub(super) fn restore_closed_codex(
        &mut self,
        thread: String,
        closed: tabs::ClosedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.call_codex_in(
            window,
            cx,
            move |service| Ok((service.resume(&thread)?, service.models()?)),
            move |this, (opened, models), window, cx| {
                this.codex_models = models;
                this.insert_codex_chat(opened, false, window, cx);
                this.update_tabs(cx, |tabs| tabs.restore_closed(&closed));
                if this.settings.is_none() {
                    this.focus_active(window, cx);
                }
            },
        );
    }

    pub(in crate::ui) fn delete_session(
        &mut self,
        key: ChatKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match key.clone() {
            ChatKey::Proteus(dir) => self.call_in(
                window,
                cx,
                move |api| api.delete_session(&dir),
                move |this, _, window, cx| this.forget_chat(key, window, cx),
            ),
            ChatKey::Codex(id) => self.call_codex_in(
                window,
                cx,
                move |service| Ok(service.delete_thread(&id)?),
                move |this, _, window, cx| this.forget_chat(key, window, cx),
            ),
        }
    }

    fn forget_chat(&mut self, key: ChatKey, window: &mut Window, cx: &mut Context<Self>) {
        let id = key.tab_id();
        self.sessions.retain(|summary| summary.key != key);
        self.opening.remove(&id);
        self.update_tabs(cx, |layout| layout.extract(&id));
        super::super::prefs::update(cx, |prefs| prefs.sessions.forget(&key));
        self.focus_active(window, cx);
    }
}
