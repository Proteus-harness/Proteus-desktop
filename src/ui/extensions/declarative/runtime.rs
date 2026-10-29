//! One extension process shared by all surfaces of the extension.

mod commands;
mod content_queue;
use content_queue::ContentQueue;
mod invalidation;
mod scopes;
use scopes::Scope;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::*;

use super::view::DeclarativeView;
use crate::declarative::manifest::Installed;
use crate::declarative::presentation::Presentation;
use crate::declarative::process::{Process, Signal};
use crate::declarative::protocol::{
    Content, HostEventKind, Incoming, Outgoing, PROTOCOL, Server, SurfaceKind,
};
use crate::ui::extensions::host::{Host, HostEvent, OpenDocument};

pub(super) struct Runtime {
    generation: u64,
    settings: VecDeque<commands::Setting>,
    setting_running: bool,
    extension: &'static Installed,
    host: Host,
    scopes: HashMap<String, Scope>,
    process: Option<Process>,
    theme_context: Option<serde_json::Value>,
    views: HashMap<String, Attached>,
    /// Why the process is not running, or its last protocol mistake.
    pub(super) error: Option<String>,
    pub(super) popup_closed: u64,
    _pump: Option<Task<()>>,
    _preferences: Subscription,
}

struct Attached {
    view: WeakEntity<DeclarativeView>,
    surface: SurfaceKind,
    argument: Option<String>,
    content: Option<Arc<Content>>,
    context: String,
    host: Host,
    window: AnyWindowHandle,
    route_generation: u64,
    /// At most one source render is in the process. `content` is the latest
    /// requested source, replacing rather than queueing intermediate edits.
    queue: ContentQueue,
}

impl Runtime {
    pub(super) fn new(
        extension: &'static Installed,
        host: Host,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            generation: 0,
            settings: VecDeque::new(),
            setting_running: false,
            extension,
            host,
            scopes: HashMap::new(),
            process: None,
            theme_context: None,
            views: HashMap::new(),
            error: None,
            popup_closed: 0,
            _pump: None,
            _preferences: cx.observe_global::<crate::ui::prefs::ClientPreferences>(|this, cx| {
                if super::super::is_enabled(this.extension_id(), cx) {
                    if this.process.is_none() && this.error.is_none() {
                        this.start(cx);
                    } else if this.process.is_some() {
                        let theme = this.content_context(cx);
                        if this.theme_context.as_ref() != Some(&theme) {
                            this.theme_context = Some(theme);
                            this.invalidate_content(cx);
                        }
                    }
                } else {
                    this.stop(cx);
                }
            }),
        }
    }

    pub(super) fn title(&self) -> &'static str {
        &self.extension.manifest.title
    }

    pub(super) fn attach(
        &mut self,
        view: String,
        surface: SurfaceKind,
        argument: Option<String>,
        content: Option<Arc<Content>>,
        entity: WeakEntity<DeclarativeView>,
        host: Host,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let context = host.scope();
        self.ensure_scope(host.clone(), window, cx);
        self.views.insert(
            view.clone(),
            Attached {
                view: entity,
                surface,
                argument,
                content,
                context,
                host,
                window,
                route_generation: 0,
                queue: ContentQueue::default(),
            },
        );
        if self.process.is_some() {
            self.render(&view, cx);
        } else if self.error.is_none() {
            self.start(cx);
        }
    }

    pub(super) fn detach(&mut self, view: &str) {
        if self.views.remove(view).is_some() {
            self.send(&Outgoing::Close { view: view.into() });
            self.prune_scopes();
        }
    }

    pub(super) fn send(&self, message: &Outgoing) {
        if let Some(process) = &self.process {
            process.send(message);
        }
    }

    /// Starts the process again, e.g. after the user fixed it.
    pub(super) fn restart(&mut self, cx: &mut Context<Self>) {
        self.stop(cx);
        self.error = None;
        self.start(cx);
        cx.notify();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if !super::super::is_enabled(self.extension_id(), cx) {
            return;
        }
        // Read the host outside this update: it lives in the workspace.
        let host = self
            .scopes
            .values()
            .next()
            .map(|scope| scope.host.clone())
            .unwrap_or_else(|| self.host.clone());
        let this = cx.weak_entity();
        cx.defer(move |cx| {
            let server = host.api(cx).map(|api| Server {
                origin: api.origin().to_string(),
                token: api.token().to_string(),
            });
            let workspace = host.workspace_path(cx);
            let _ = this.update(cx, |this, cx| this.launch(server, workspace, cx));
        });
    }

    pub(super) fn reload(&mut self, cx: &mut Context<Self>) {
        let current = super::installed(self.extension_id());
        if current.is_some_and(|extension| std::ptr::eq(extension, self.extension)) {
            return;
        }
        self.stop(cx);
        match current {
            Some(extension) => {
                self.extension = extension;
                self.error = None;
                self.start(cx);
            }
            None => self.error = Some("расширение удалено".into()),
        }
        cx.notify();
    }

    fn launch(
        &mut self,
        server: Option<Server>,
        workspace: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        if self.process.is_some() || !super::super::is_enabled(self.extension_id(), cx) {
            return;
        }
        let (Some(server), Some(workspace)) = (server, workspace) else {
            // Not connected yet; the next host event starts it.
            return;
        };
        match Process::spawn(self.extension) {
            Ok((process, signals)) => {
                process.send(&Outgoing::Hello {
                    protocol: PROTOCOL,
                    features: crate::declarative::protocol::FEATURES
                        .iter()
                        .map(|feature| (*feature).into())
                        .collect(),
                    workspace,
                    server,
                });
                self.process = Some(process);
                let contexts: Vec<_> = self.scopes.keys().cloned().collect();
                for context in contexts {
                    self.announce_scope(&context, cx);
                }
                self._pump = Some(cx.spawn(async move |this, cx| {
                    while let Ok(signal) = signals.recv().await {
                        if this.update(cx, |this, cx| this.handle(signal, cx)).is_err() {
                            break;
                        }
                    }
                }));
                let views: Vec<String> = self.views.keys().cloned().collect();
                for view in views {
                    self.render(&view, cx);
                }
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    fn render(&mut self, view: &str, cx: &App) {
        let theme = self.content_context(cx);
        self.theme_context = Some(theme.clone());
        let Some(attached) = self.views.get_mut(view) else {
            return;
        };
        let latest = attached.content.as_deref();
        let materialize = || {
            latest.cloned().map(|mut content| {
                if !content.context.is_object() {
                    content.context = serde_json::json!({});
                }
                content.context["theme"] = theme["theme"].clone();
                content
            })
        };
        let content = if attached.surface == SurfaceKind::Content {
            let revision = latest.and_then(|content| content.source_revision);
            let Some(content) = attached.queue.begin_with(revision, materialize) else {
                return;
            };
            content
        } else {
            materialize()
        };
        let message = Outgoing::Render {
            context: attached.context.clone(),
            view: view.into(),
            surface: attached.surface,
            argument: attached.argument.clone(),
            content,
            session_dir: self
                .scopes
                .get(&attached.context)
                .and_then(|scope| scope.host.session_dir(cx)),
        };
        self.send(&message);
    }

    pub(super) fn set_content(&mut self, view: &str, content: Arc<Content>, cx: &App) {
        if let Some(attached) = self.views.get_mut(view) {
            attached.content = Some(content);
        }
        if self.process.is_some() {
            self.render(view, cx);
        }
    }

    fn content_finished(&mut self, view: &str, revision: Option<u64>, cx: &App) {
        let Some(attached) = self.views.get_mut(view) else {
            return;
        };
        if attached.surface == SurfaceKind::Content
            && attached.queue.finish(
                revision,
                attached
                    .content
                    .as_ref()
                    .and_then(|content| content.source_revision),
            )
        {
            self.render(view, cx);
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.settings.clear();
        self.popup_closed += 1;
        self.process = None;
        self._pump = None;
        self.reset_scopes();
        for attached in self.views.values() {
            if let Some(view) = attached.view.upgrade() {
                view.update(cx, |view, cx| view.reset(cx));
            }
        }
        for attached in self.views.values_mut() {
            attached.queue.reset();
        }
    }

    fn handle(&mut self, signal: Signal, cx: &mut Context<Self>) {
        match signal {
            Signal::Message(Incoming::InvalidateContent {}) => {
                let extension = self.extension_id();
                // The sender is one of the targets. Drop its active entity
                // borrow before walking the registry and updating runtimes.
                cx.defer(move |cx| invalidation::broadcast(extension, cx));
            }
            Signal::Message(Incoming::ContentError {
                view,
                session_dir,
                error,
                sequence,
                source_revision,
            }) => {
                if session_dir != self.host_for(&view).and_then(|host| host.session_dir(cx)) {
                    return;
                }
                if let Some(entity) = self
                    .views
                    .get(&view)
                    .filter(|attached| attached.surface == SurfaceKind::Content)
                    .and_then(|attached| attached.view.upgrade())
                {
                    entity.update(cx, |view, cx| {
                        view.fail_with_revision(error, sequence, source_revision, cx)
                    });
                }
                self.content_finished(&view, source_revision, cx);
            }
            Signal::Message(Incoming::Focus {
                view: view_id,
                control,
            }) => {
                if let Some(view) = self
                    .views
                    .get(&view_id)
                    .and_then(|attached| attached.view.upgrade())
                {
                    let Some(attached) = self.views.get(&view_id) else {
                        return;
                    };
                    let _ = attached.window.update(cx, move |_, window, cx| {
                        if let Some(control) = control {
                            view.update(cx, |view, cx| view.focus_control(&control, window, cx));
                        } else {
                            view.read(cx).focus().focus(window, cx);
                        }
                    });
                }
            }
            Signal::Message(Incoming::ClosePopup { context }) => {
                if let Some(scope) = self.scopes.get_mut(&context) {
                    scope.popup_closed += 1;
                }
            }
            Signal::Message(Incoming::Clipboard { text }) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text))
            }
            Signal::Message(Incoming::SetSession {
                context,
                view,
                id,
                session_dir,
                setting,
            }) => {
                let context = view
                    .as_ref()
                    .and_then(|view| self.views.get(view))
                    .map_or(context, |view| view.context.clone());
                self.set_session(context, view, id, session_dir, setting, cx);
            }
            Signal::Message(Incoming::Badge {
                hidden,
                view,
                session_dir,
                title,
                lines,
                face,
            }) => {
                if session_dir != self.host_for(&view).and_then(|host| host.session_dir(cx)) {
                    return;
                }
                let Some(attached) = self
                    .views
                    .get(&view)
                    .filter(|view| view.surface == SurfaceKind::Badge)
                else {
                    self.error = Some("badge message requires an attached badge surface".into());
                    cx.notify();
                    return;
                };
                let narrow = self
                    .extension
                    .manifest
                    .surfaces
                    .badge
                    .as_ref()
                    .is_some_and(|badge| badge.narrow);
                if narrow && !face.narrow() {
                    self.error = Some("a narrow badge cannot display text".into());
                } else if let Some(entity) = attached.view.upgrade() {
                    entity.update(cx, |view, cx| {
                        view.present(
                            Some(Presentation {
                                hidden,
                                title,
                                lines,
                                face,
                            }),
                            cx,
                        )
                    });
                    self.error = None;
                }
            }
            Signal::Message(Incoming::View {
                source_revision,
                title,
                keys,
                view,
                root,
                sequence,
                session_dir,
            }) => {
                if session_dir != self.host_for(&view).and_then(|host| host.session_dir(cx)) {
                    return;
                }
                let narrow = self
                    .extension
                    .manifest
                    .surfaces
                    .badge
                    .as_ref()
                    .is_some_and(|badge| badge.narrow);
                if narrow
                    && self
                        .views
                        .get(&view)
                        .is_some_and(|attached| attached.surface == SurfaceKind::Badge)
                {
                    self.error = Some("narrow badges require a badge presentation".into());
                    cx.notify();
                    return;
                }
                if let Some(entity) = self.views.get(&view).and_then(|a| a.view.upgrade()) {
                    let accepted = entity.update(cx, |view, cx| {
                        view.show_with_revision(root, sequence, keys, source_revision, cx)
                    });
                    if accepted
                        && let Some(title) = title
                        && let Some(attached) = self.views.get(&view)
                        && attached.surface == SurfaceKind::Document
                        && let Some(argument) = &attached.argument
                        && let Some(host) = self.host_for(&view)
                    {
                        host.document_title(argument, title, cx);
                    }
                }
                self.content_finished(&view, source_revision, cx);
                self.error = None;
            }
            Signal::Message(Incoming::OpenDocument {
                mut context,
                view,
                argument,
                preview,
                focus,
            }) => {
                if self.extension.manifest.surfaces.document.is_none() {
                    self.error = Some("расширение не объявило документы".into());
                } else {
                    if let Some(view) = &view {
                        let Some(attached) = self.views.get(view) else {
                            return;
                        };
                        context = attached.context.clone();
                    }
                    let Some(scope) = self.scopes.get(&context) else {
                        return;
                    };
                    let (host, id) = (scope.host.clone(), self.extension_id());
                    let _ = scope.window.update(cx, move |_, window, cx| {
                        let open = OpenDocument { preview, focus };
                        host.open_document(id, &argument, open, window, cx);
                    });
                }
            }
            Signal::Invalid(error) => self.error = Some(format!("неверное сообщение: {error}")),
            Signal::Exited(text) => {
                self.process = None;
                self.error = Some(text);
            }
        }
        cx.notify();
    }

    fn extension_id(&self) -> &'static str {
        self.extension.manifest.id.as_str()
    }
}
