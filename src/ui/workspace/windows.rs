//! Адаптировано из Zed: crates/workspace/src/workspace.rs (new window),
//! crates/gpui/examples/move_entity_between_windows.rs (entity transfer).
//! Windows share one server lease, but own independent layouts and focus.

use std::cell::RefCell;
use std::rc::Rc;

use gpui_kit::*;

use super::{Status, Workspace};
use crate::connection::BackendGuard;
use crate::tabs::{self, TabLayout};
use crate::ui::{launcher::Launcher, prefs};

#[derive(Default, Clone)]
pub(in crate::ui) struct WindowGroup(Rc<RefCell<Group>>);

#[derive(Default)]
struct Group {
    backend: Option<BackendGuard>,
    windows: Vec<(WeakEntity<Workspace>, TabLayout)>,
    quitting: bool,
}

impl WindowGroup {
    pub(super) fn register(&self, view: WeakEntity<Workspace>, layout: TabLayout) {
        self.0.borrow_mut().windows.push((view, layout));
    }

    pub(super) fn set_backend(&self, backend: Option<BackendGuard>) {
        self.0.borrow_mut().backend = backend;
    }

    pub(super) fn take_backend(&self) -> Option<BackendGuard> {
        let mut group = self.0.borrow_mut();
        group.quitting = true;
        group.backend.take()
    }

    pub(in crate::ui) fn all_tabs(&self) -> TabLayout {
        self.0.borrow().saved_layout().unwrap_or_default()
    }

    pub(super) fn watch_backend(
        &self,
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
                cx.update(|cx| WindowGroup(group).server_stopped(&report, cx));
            }
        })
        .detach();
    }

    fn other_tab(&self, source: EntityId, id: &str) -> Option<WeakEntity<Workspace>> {
        self.0
            .borrow()
            .windows
            .iter()
            .find(|(view, layout)| view.entity_id() != source && layout.contains(id))
            .map(|(view, _)| view.clone())
    }

    pub(super) fn server_stopped(&self, report: &str, cx: &mut App) {
        let views: Vec<_> = self
            .0
            .borrow()
            .windows
            .iter()
            .map(|(view, _)| view.clone())
            .collect();
        for view in views {
            view.update(cx, |this, cx| {
                this.status = Status::Failed(report.to_owned().into());
                cx.notify();
            })
            .ok();
        }
    }

    pub(in crate::ui) fn persist(&self, id: EntityId, layout: &TabLayout, cx: &mut App) {
        let saved = {
            let mut group = self.0.borrow_mut();
            if let Some((_, saved)) = group
                .windows
                .iter_mut()
                .find(|(view, _)| view.entity_id() == id)
            {
                *saved = layout.clone();
            }
            group.saved_layout()
        };
        if let Some(saved) = saved {
            prefs::update(cx, |prefs| prefs.tabs = saved);
        }
    }

    pub(super) fn release(&self, id: EntityId, cx: &mut App) {
        let saved = {
            let mut group = self.0.borrow_mut();
            group.windows.retain(|(view, _)| view.entity_id() != id);
            if group.quitting {
                None
            } else {
                group.saved_layout()
            }
        };
        // Keep the last window's saved tabs for restart. Closing one window
        // removes only its tabs while the remaining windows keep their server.
        if let Some(saved) = saved {
            prefs::update(cx, |prefs| prefs.tabs = saved);
        }
    }
}

impl Group {
    fn saved_layout(&self) -> Option<TabLayout> {
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
            .other_tab(cx.entity_id(), id)
            .and_then(|view| view.upgrade())
        else {
            return false;
        };
        let handle = view.read(cx).window_handle;
        handle
            .update(cx, |_, window, cx| {
                view.update(cx, |this, cx| this.show_tab_in(id, None, window, cx));
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
        if !self.layout.contains(id) || self.api.is_none() {
            return;
        }
        if !tabs::is_chat(id) && self.ensure_document(id, window, cx).is_none() {
            return;
        }
        let (path, api, target, group, sessions) = (
            self.workspace_path.clone(),
            self.api.clone(),
            self.target.clone(),
            self.windows.clone(),
            self.sessions.clone(),
        );
        let root_target = target.clone();
        let opened = gpui_kit::open_window(crate::window_options(cx), cx, move |window, cx| {
            let view = cx.new(|cx| {
                let mut workspace =
                    Self::empty(path, target, TabLayout::default(), group, window, cx);
                workspace.api = api;
                workspace.sessions = sessions;
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
                        if let Some(dir) = tabs::chat_dir(id)
                            && !this.chats.contains_key(id)
                        {
                            this.ensure_chat(dir.to_path_buf(), window, cx);
                        }
                        this.focus_active(window, cx);
                        window.activate_window();
                    });
            })
            .expect("newly created tab window is live");
    }
}
