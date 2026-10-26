use std::cell::Cell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use anyhow::Result;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppSessionSummary;

use super::chats::OpenChat;
use super::chrome::WorkspaceRequest;
use super::extensions::{OpenDocument, SurfaceView};
use super::panels::{Renaming, SessionList};
use super::settings::SettingsView;
use super::tabs::{SplitResize, TabDrop, TabMotion};
use crate::api::Api;
use crate::attention::Tracker;
use crate::connection::BackendGuard;
use crate::launch::LaunchOptions;
use crate::tabs::PaneId;

pub(super) mod startup;
use startup::{Boot, boot};

pub(super) enum Status {
    Starting,
    Ready,
    Failed(SharedString),
}

impl EventEmitter<WorkspaceRequest> for Workspace {}

pub struct Workspace {
    pub(super) workspace_path: PathBuf,
    pub(super) status: Status,
    api: Option<Api>,
    backend: Option<BackendGuard>,
    pub(super) sessions: Vec<AppSessionSummary>,
    /// When the session list first arrived, so its rows can enter in turn.
    pub(super) sessions_arrived: Option<Instant>,
    /// A session list request is running; `sessions_stale` asks for
    /// another one when it ends.
    pub(super) sessions_loading: bool,
    pub(super) sessions_stale: bool,
    /// Activity news of every session, for system notifications.
    pub(super) attention: Tracker,
    pub(super) window_handle: AnyWindowHandle,
    /// Archived sessions are listed.
    pub(super) show_archived: bool,
    /// A session name being edited in the list.
    pub(super) renaming: Option<Renaming>,
    /// Search and keyboard cursor of the session list.
    pub(super) session_list: SessionList,
    /// Open chat tabs by tab id.
    pub(super) chats: HashMap<String, OpenChat>,
    pub(super) startup_streams:
        HashMap<PathBuf, async_channel::Receiver<crate::connection::PumpItem>>,
    /// Chats being resumed, and how they open.
    pub(super) opening: HashMap<String, Rc<Cell<Option<OpenDocument>>>>,
    /// Tab id of the chat that commands and extensions address.
    pub(super) active_chat: Option<String>,
    /// Error of a workspace call while no chat is open.
    pub(super) notice: Option<SharedString>,
    pub(super) window_title: String,
    pub(super) settings: Option<Entity<SettingsView>>,
    pub(super) settings_subscription: Option<Subscription>,
    pub(super) navigation: crate::navigation::History<super::navigation::Destination>,
    /// Extension panels, document tabs and icons built so far; the tab
    /// layout and the side panel themselves are client preferences.
    pub(super) panels: HashMap<&'static str, SurfaceView>,
    pub(super) documents: HashMap<String, SurfaceView>,
    pub(super) document_titles: HashMap<String, SharedString>,
    pub(super) extension_processes: HashMap<&'static str, AnyEntity>,
    pub(super) badges: HashMap<&'static str, AnyView>,
    pub(super) header_bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(super) rail_bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Size of the split whose divider is being dragged.
    pub(super) split_drag: Option<SplitResize>,
    /// Width of the side panel while its edge is dragged.
    pub(super) sidebar_drag: Option<f32>,
    /// The panel the side panel showed last, kept on screen while it
    /// slides closed.
    pub(super) sidebar_last: Option<&'static str>,
    /// A tab is being dragged, so panes show where it can go.
    pub(super) tab_drag: bool,
    /// Where the dragged tab would land when dropped onto a pane.
    pub(super) tab_drop: Option<TabDrop>,
    /// Scroll of each pane's tab strip and the active tab it last showed,
    /// so a newly active tab is scrolled into view.
    pub(super) tab_strips: HashMap<PaneId, (ScrollHandle, Option<String>)>,
    /// Tabs opening and closing, panes moving to new places.
    pub(super) tab_motion: TabMotion,
    /// Size of the tab area at the last paint.
    pub(super) tab_area: Rc<Cell<Option<Size<Pixels>>>>,
    pub(super) focus_handle: FocusHandle,
    _startup: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(
        options: LaunchOptions,
        preparation: Option<startup::Preparation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = vec![cx.on_app_quit(|this, _| {
            let backend = this.backend.take();
            async move {
                if let Some(backend) = backend {
                    backend.shutdown();
                }
            }
        })];
        let (session_list, list_events) = SessionList::new(window, cx);
        subscriptions.push(list_events);

        let workspace_path = options.workspace.clone();
        let saved = super::prefs::current(cx).tabs.clone();
        let prepared =
            preparation.and_then(|preparation| preparation.for_workspace(&workspace_path));
        let ready = prepared
            .as_ref()
            .and_then(|prepared| prepared.try_recv().ok());
        let startup = if let Some(ready) = ready {
            // Deferred entity update runs before its first paint, rather than
            // awaiting a task after drawing an empty workspace first.
            cx.defer_in(window, move |this, window, cx| {
                this.finish_startup(ready, window, cx)
            });
            None
        } else {
            Some(cx.spawn_in(window, async move |this, cx| {
                let result = if let Some(prepared) = prepared {
                    prepared
                        .recv()
                        .await
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("startup preparation stopped")))
                } else {
                    cx.background_executor()
                        .spawn(async move { boot(&options, saved) })
                        .await
                };
                this.update_in(cx, |this, window, cx| {
                    this.finish_startup(result, window, cx)
                })
                .ok();
            }))
        };

        Self {
            workspace_path,
            status: Status::Starting,
            api: None,
            backend: None,
            sessions: Vec::new(),
            sessions_loading: false,
            sessions_stale: false,
            attention: Tracker::default(),
            window_handle: window.window_handle(),
            show_archived: false,
            renaming: None,
            session_list,
            chats: HashMap::new(),
            startup_streams: HashMap::new(),
            opening: HashMap::new(),
            active_chat: None,
            notice: None,
            window_title: String::new(),
            settings: None,
            settings_subscription: None,
            navigation: crate::navigation::History::default(),
            panels: HashMap::new(),
            documents: HashMap::new(),
            document_titles: HashMap::new(),
            extension_processes: HashMap::new(),
            badges: HashMap::new(),
            header_bounds: Rc::default(),
            rail_bounds: Rc::default(),
            split_drag: None,
            sidebar_drag: None,
            sidebar_last: None,
            tab_drag: false,
            tab_drop: None,
            tab_strips: HashMap::new(),
            sessions_arrived: None,
            tab_motion: TabMotion::default(),
            tab_area: Rc::default(),
            focus_handle: cx.focus_handle(),
            _startup: startup,
            _subscriptions: subscriptions,
        }
    }

    fn finish_startup(
        &mut self,
        result: Result<Boot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::startup_trace::mark("workspace.boot.applied");
        match result {
            Ok(boot) => {
                self.api = Some(boot.api);
                if let Some(exited) = boot.backend.as_ref().map(|guard| guard.exited.clone()) {
                    cx.spawn(async move |this, cx| {
                        let Ok(report) = exited.recv().await else {
                            return;
                        };
                        this.update(cx, |this, cx| {
                            this.status = Status::Failed(report.into());
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                self.backend = boot.backend;
                self.startup_streams = boot.streams;
                self.sessions = boot.sessions;
                self.sessions_arrived = (!self.sessions.is_empty()).then(Instant::now);
                self.status = Status::Ready;
                self.restore_chats(boot.restored, boot.resumed, window, cx);
                if let Some(dir) = boot.startup {
                    self.add_chat(dir, None, window, cx);
                } else {
                    self.focus_active_chat(window, cx);
                }
                self.refresh_sessions(cx);
            }
            Err(error) => self.status = Status::Failed(format!("{error:#}").into()),
        }
        cx.notify();
    }

    pub(super) fn api(&self) -> Option<Api> {
        self.api.clone()
    }

    /// Chats whose agent run is still going; closing the server ends them.
    pub(super) fn running_chats(&self, cx: &App) -> usize {
        self.chats
            .values()
            .filter(|chat| chat.view.read(cx).session.running())
            .count()
    }

    /// Runs a blocking server call off the UI thread.
    pub(super) fn call<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Api) -> Result<R> + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        let Some(api) = self.api.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { job(&api) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(value) => done(this, value, cx),
                    Err(error) => this.report(format!("{error:#}"), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Like [`Self::call`], for results that need the window.
    pub(super) fn call_in<R: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Api) -> Result<R> + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(api) = self.api.clone() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { job(&api) })
                .await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(value) => done(this, value, window, cx),
                    Err(error) => this.report(format!("{error:#}"), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
