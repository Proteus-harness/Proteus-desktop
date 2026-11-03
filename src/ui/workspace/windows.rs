//! Адаптировано из Zed: crates/workspace/src/workspace.rs (new window),
//! crates/gpui/examples/move_entity_between_windows.rs (entity transfer).
//! Windows share one server lease, but own independent layouts and focus.

use super::drivers::CoreConnection;
use crate::agents::codex::{CatalogEvent, CodexService};
use crate::api::Api;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use gpui_kit::*;

use super::{Status, Workspace};
use crate::connection::BackendGuard;
use crate::preferences::TabWorkspace;
use crate::tabs::{self, ChatSpaces, TabLayout};
use crate::ui::{launcher::Launcher, prefs};

#[cfg(test)]
#[path = "windows/tests.rs"]
mod tests;

#[derive(Default, Clone)]
pub(in crate::ui) struct WindowGroup(Rc<RefCell<Group>>);

#[derive(Default)]
struct Group {
    owner: Option<TabWorkspace>,
    core: Arc<Mutex<Option<CoreConnection>>>,
    codex: Arc<Mutex<Option<CodexService>>>,
    windows: Vec<(WeakEntity<Workspace>, ChatSpaces)>,
    quitting: bool,
}

impl WindowGroup {
    pub(super) fn new(workspace: &std::path::Path, target: &crate::launch::Target) -> Self {
        Self(Rc::new(RefCell::new(Group {
            owner: TabWorkspace::new(workspace, target),
            ..Group::default()
        })))
    }

    pub(super) fn register(&self, view: WeakEntity<Workspace>, layout: ChatSpaces) {
        self.0.borrow_mut().windows.push((view, layout));
    }

    pub(super) fn set_core(&self, api: Api, backend: Option<BackendGuard>) {
        *self.0.borrow().core.lock().unwrap() = Some(CoreConnection { api, backend });
    }

    pub(super) fn has_core_connection(&self, connection: u64) -> bool {
        self.0.borrow().core.try_lock().ok().is_some_and(|core| {
            core.as_ref()
                .is_some_and(|core| core.api.connection_id() == connection)
        })
    }

    pub(super) fn core_slot(&self) -> Arc<Mutex<Option<CoreConnection>>> {
        self.0.borrow().core.clone()
    }

    pub(in crate::ui) fn codex_slot(&self) -> Arc<Mutex<Option<CodexService>>> {
        self.0.borrow().codex.clone()
    }

    pub(super) fn set_codex(&self, service: CodexService, cx: &mut Context<Workspace>) {
        self.watch_codex(&service, cx);
        *self.0.borrow().codex.lock().unwrap() = Some(service);
    }

    /// One catalog consumer per server, independent of any open chat. Native
    /// deletion notifications also cover descendants and chats in other windows.
    pub(super) fn watch_codex(&self, service: &CodexService, cx: &mut Context<Workspace>) {
        let Some(events) = service.take_catalog_events() else {
            return;
        };
        let weak = Rc::downgrade(&self.0);
        cx.spawn(async move |_, cx| {
            while let Ok(first) = events.recv().await {
                let mut deleted = std::collections::HashSet::new();
                let CatalogEvent::ThreadDeleted { id } = first;
                deleted.insert(crate::tabs::ChatKey::Codex(id));
                for _ in 1..256 {
                    let Ok(CatalogEvent::ThreadDeleted { id }) = events.try_recv() else {
                        break;
                    };
                    deleted.insert(crate::tabs::ChatKey::Codex(id));
                }
                let Some(group) = weak.upgrade() else {
                    return;
                };
                cx.update(|cx| {
                    let views: Vec<_> = group
                        .borrow()
                        .windows
                        .iter()
                        .map(|(view, _)| view.clone())
                        .collect();
                    // Store removal once, rather than once per native window.
                    prefs::update(cx, |prefs| {
                        for key in &deleted {
                            prefs.sessions.forget(key);
                        }
                    });
                    for view in views {
                        let Ok(handle) = view.read_with(cx, |this, _| this.window_handle) else {
                            continue;
                        };
                        handle
                            .update(cx, |_, window, cx| {
                                view.update(cx, |this, cx| this.forget_chats(&deleted, window, cx))
                                    .ok();
                            })
                            .ok();
                    }
                });
                super::super::chat::yield_after_batch().await;
            }
        })
        .detach();
    }

    pub(super) fn take_backend(&self) -> Option<BackendGuard> {
        let mut group = self.0.borrow_mut();
        group.quitting = true;
        group
            .core
            .lock()
            .unwrap()
            .take()
            .and_then(|core| core.backend)
    }

    /// A successful Core delete addresses the same retained root in every
    /// native window, including document-only detached fragments.
    pub(in crate::ui) fn forget_elsewhere(
        &self,
        source: EntityId,
        keys: &std::collections::HashSet<crate::tabs::ChatKey>,
        cx: &mut App,
    ) {
        let views: Vec<_> = self
            .0
            .borrow()
            .windows
            .iter()
            .filter(|(view, _)| view.entity_id() != source)
            .map(|(view, _)| view.clone())
            .collect();
        for view in views {
            let Ok(handle) = view.read_with(cx, |this, _| this.window_handle) else {
                continue;
            };
            handle
                .update(cx, |_, window, cx| {
                    view.update(cx, |this, cx| this.forget_chats(keys, window, cx))
                        .ok();
                })
                .ok();
        }
    }

    pub(in crate::ui) fn all_tabs(&self) -> TabLayout {
        let mut all = TabLayout::default();
        if let Some(spaces) = self.0.borrow().saved_layout() {
            for id in spaces.all_tabs() {
                all.open(id, |_| false);
            }
        }
        all
    }

    pub(super) fn watch_backend(
        &self,
        connection: u64,
        exited: async_channel::Receiver<String>,
        cx: &mut Context<Workspace>,
    ) {
        // The waiting task must not retain the server it is waiting for:
        // the last window dropping its lease still stops the backend.
        let weak = Rc::downgrade(&self.0);
        cx.spawn(async move |_, cx| {
            let Ok(report) = exited.recv().await else {
                return;
            };
            if let Some(group) = weak.upgrade() {
                cx.update(|cx| WindowGroup(group).server_stopped(connection, &report, cx));
            }
        })
        .detach();
    }

    fn other_tab(
        &self,
        source: EntityId,
        id: &str,
        root: Option<&str>,
    ) -> Option<WeakEntity<Workspace>> {
        self.0
            .borrow()
            .windows
            .iter()
            .find(|(view, spaces)| {
                view.entity_id() != source
                    && if tabs::is_chat(id) {
                        spaces.contains(id)
                    } else if let Some(root) = root {
                        spaces
                            .root_layouts()
                            .any(|(owner, layout)| owner == root && layout.contains(id))
                    } else {
                        spaces.active_root().is_none() && spaces.active_layout().contains(id)
                    }
            })
            .map(|(view, _)| view.clone())
    }

    // Only the owned process watcher may retire an app-wide Core lease.
    // Session stream shutdown is local to its chat.
    fn server_stopped(&self, connection: u64, report: &str, cx: &mut App) {
        let core = self.0.borrow().core.clone();
        {
            let Ok(mut current) = core.try_lock() else {
                return;
            };
            if current
                .as_ref()
                .is_none_or(|core| core.api.connection_id() != connection)
            {
                return;
            }
            current.take();
        }
        let views: Vec<_> = self
            .0
            .borrow()
            .windows
            .iter()
            .map(|(view, _)| view.clone())
            .collect();
        for view in views {
            view.update(cx, |this, cx| {
                if this
                    .api
                    .as_ref()
                    .is_none_or(|api| api.connection_id() != connection)
                {
                    return;
                }
                this.api = None;
                this.notice = Some(report.to_owned().into());
                // A Core failure cannot hide another driver's running turn or Stop.
                this.status = Status::Ready;
                cx.notify();
            })
            .ok();
        }
    }

    pub(in crate::ui) fn persist(&self, id: EntityId, layout: &ChatSpaces, cx: &mut App) {
        let saved = {
            let mut group = self.0.borrow_mut();
            if let Some((_, saved)) = group
                .windows
                .iter_mut()
                .find(|(view, _)| view.entity_id() == id)
            {
                *saved = layout.clone();
            }
            group.saved_layout().zip(group.owner.clone())
        };
        if let Some((saved, owner)) = saved {
            prefs::update(cx, |prefs| {
                prefs.tabs = saved;
                prefs.tabs_workspace = Some(owner);
            });
        }
    }

    pub(super) fn release(&self, id: EntityId, cx: &mut App) {
        let saved = {
            let mut group = self.0.borrow_mut();
            group.windows.retain(|(view, _)| view.entity_id() != id);
            if group.quitting {
                None
            } else {
                group.saved_layout().zip(group.owner.clone())
            }
        };
        // Keep the last window's saved tabs for restart. Closing one window
        // removes only its tabs while the remaining windows keep their server.
        if let Some((saved, owner)) = saved {
            prefs::update(cx, |prefs| {
                prefs.tabs = saved;
                prefs.tabs_workspace = Some(owner);
            });
        }
    }
}

impl Group {
    fn saved_layout(&self) -> Option<ChatSpaces> {
        let mut windows = self.windows.iter();
        let mut saved = windows.next()?.1.clone();
        for (_, layout) in windows {
            saved.include_window(layout);
        }
        Some(saved)
    }
}

impl Workspace {
    /// Selecting a transferred session activates its existing window instead
    /// of retaining a second transcript and event stream for the same chat.
    pub(in crate::ui) fn show_external_tab(&self, id: &str, cx: &mut Context<Self>) -> bool {
        let Some(view) = self
            .windows
            .other_tab(cx.entity_id(), id, self.spaces.active_root())
            .and_then(|view| view.upgrade())
        else {
            return false;
        };
        let handle = view.read(cx).window_handle;
        let requested_root = self.spaces.active_root().map(str::to_owned);
        handle
            .update(cx, |_, window, cx| {
                view.update(cx, |this, cx| {
                    let root = if tabs::is_chat(id) {
                        tabs::chat_key(id).map(|key| this.chat_space_root(&key))
                    } else {
                        requested_root
                    };
                    if let Some(root) = root {
                        this.select_space_root(&root, cx);
                    }
                    this.show_tab_in(id, None, window, cx);
                });
                window.activate_window();
            })
            .is_ok()
    }

    /// Move the existing entity only after the platform created a new window.
    /// No boot, session writer or additional transcript cache is created.
    pub(in crate::ui) fn detach_tab(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.layout.contains(id) {
            return;
        }
        if !tabs::is_chat(id) && self.ensure_document(id, window, cx).is_none() {
            return;
        }
        let (path, api, target, group, sessions, session_projects, codex_models) = (
            self.workspace_path.clone(),
            self.api.clone(),
            self.target.clone(),
            self.windows.clone(),
            self.sessions.clone(),
            self.session_projects.clone(),
            self.codex_models.clone(),
        );
        let root_target = target.clone();
        let mut detached_spaces = ChatSpaces::default();
        detached_spaces.set_active(self.spaces.active_root(), TabLayout::default());
        let opened = gpui_kit::open_window(crate::window_options(cx), cx, move |window, cx| {
            let view = cx.new(|cx| {
                let mut workspace = Self::empty(path, target, detached_spaces, group, window, cx);
                workspace.api = api;
                workspace.sessions = sessions;
                workspace.session_projects = session_projects;
                workspace.codex_models = codex_models;
                workspace.status = Status::Ready;
                workspace
            });
            cx.new(|cx| Launcher::from_workspace(view, root_target, window, cx))
        });
        let (handle, launcher) = match opened {
            Ok(opened) => opened,
            Err(error) => return self.report(format!("Не удалось открыть окно: {error:#}"), cx),
        };
        let pinned = self.layout.is_pinned(id);
        let chat = self.chats.remove(id);
        let document = self.documents.remove(id);
        let title = self.document_titles.remove(id);
        let stream = tabs::chat_dir(id).and_then(|dir| self.startup_streams.remove(dir));
        self.opening.remove(id);
        self.update_tabs(cx, |layout| layout.extract(id));
        self.focus_active(window, cx);
        // The newly created window is still live, and this update happens in
        // the same event turn: no frame can observe the entity in both roots.
        handle
            .update(cx, |_, window, cx| {
                launcher
                    .read(cx)
                    .workspace()
                    .clone()
                    .update(cx, |this, cx| {
                        if let Some(chat) = chat {
                            this.adopt_chat(id, chat.view, window, cx);
                        }
                        if let Some(document) = document {
                            document.rehost(cx.weak_entity(), window, cx);
                            this.documents.insert(id.to_owned(), document);
                        }
                        if let Some(title) = title {
                            this.document_titles.insert(id.to_owned(), title);
                        }
                        if let Some(stream) = stream
                            && let Some(dir) = tabs::chat_dir(id)
                        {
                            this.startup_streams.insert(dir.to_path_buf(), stream);
                        }
                        this.update_tabs(cx, |layout| {
                            layout.open(id, |_| false);
                            layout.keep(id);
                            layout.set_pinned(id, pinned);
                        });
                        if let Some(key) = tabs::chat_key(id)
                            && !this.chats.contains_key(id)
                        {
                            this.ensure_chat(key, window, cx);
                        }
                        this.focus_active(window, cx);
                        window.activate_window();
                    });
            })
            .expect("newly created tab window is live");
    }
}
