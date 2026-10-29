//! Window routes of a shared process. A move changes only one attachment;
//! subscription lifetimes follow the windows that still own its surfaces.

use super::*;
use crate::ui::chat::Chat;
use crate::ui::workspace::Workspace;
use gpui_kit::component::*;

pub(super) struct Scope {
    pub(super) host: Host,
    pub(super) window: AnyWindowHandle,
    pub(super) popup_closed: u64,
    source: Option<(WeakEntity<Chat>, PathBuf)>,
    session: Option<PathBuf>,
    announced: bool,
    _events: Option<Subscription>,
    _session_events: Option<Subscription>,
}

impl Runtime {
    pub(in crate::ui::extensions::declarative) fn host_for(&self, view: &str) -> Option<&Host> {
        let view = self.views.get(view)?;
        Some(
            self.scopes
                .get(&view.context)
                .map_or(&view.host, |scope| &scope.host),
        )
    }

    pub(in crate::ui::extensions::declarative) fn popup_closed(&self, view: &str) -> u64 {
        self.popup_closed
            + self
                .views
                .get(view)
                .and_then(|view| self.scopes.get(&view.context))
                .map_or(0, |scope| scope.popup_closed)
    }

    pub(super) fn ensure_scope(
        &mut self,
        host: Host,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let context = host.scope();
        if self.scopes.contains_key(&context) {
            return;
        }
        let events_context = context.clone();
        let events = host.subscribe(cx, move |this, event, cx| {
            this.scope_event(&events_context, event, cx)
        });
        let session_context = context.clone();
        let session_events = host.subscribe_session(cx, move |this: &mut Self, event, _| {
            if this.extension.manifest.session_events {
                this.send(&Outgoing::SessionUpdate {
                    context: session_context.clone(),
                    session_dir: event.session_dir.clone(),
                    update: event.update.clone(),
                });
            }
        });
        let source = host.session_source(cx);
        self.scopes.insert(
            context.clone(),
            Scope {
                host,
                window,
                popup_closed: 0,
                source,
                session: None,
                announced: false,
                _events: events,
                _session_events: session_events,
            },
        );
        if self.process.is_some() {
            self.announce_scope(&context, cx);
        }
    }

    pub(super) fn announce_scope(&mut self, context: &str, cx: &App) {
        let Some(scope) = self.scopes.get(context) else {
            return;
        };
        let host = scope.host.clone();
        let session = host.session_dir(cx);
        let snapshot_known = self
            .scopes
            .iter()
            .any(|(id, scope)| id != context && scope.announced && scope.session == session);
        self.send(&Outgoing::Context {
            context: context.into(),
            workspace: host.workspace_path(cx).unwrap_or_default(),
            session_dir: session.clone(),
        });
        if self.extension.manifest.session_events && !snapshot_known {
            self.send(&host.session_snapshot(cx));
        }
        if let Some(scope) = self.scopes.get_mut(context) {
            scope.session = session;
            scope.announced = true;
        }
    }

    fn scope_event(&mut self, context: &str, event: HostEvent, cx: &mut Context<Self>) {
        let Some(scope) = self.scopes.get_mut(context) else {
            return;
        };
        if event == HostEvent::SessionOpened {
            // Keep the last explicitly chosen session as the fallback if its
            // tab later closes. Neither fallback holds a Chat entity alive.
            if let Some(source) = scope.host.session_source(cx) {
                scope.source = Some(source.clone());
                scope.host.inherit(Some(source));
            }
        }
        let host = scope.host.clone();
        let session_changed = scope.session != host.session_dir(cx);
        let event = match event {
            HostEvent::SessionOpened => HostEventKind::SessionOpened,
            HostEvent::RunSettled => HostEventKind::RunSettled,
            HostEvent::NavigationChanged => HostEventKind::NavigationChanged,
        };
        if self.process.is_none() && self.error.is_none() {
            self.start(cx);
        }
        if event == HostEventKind::SessionOpened {
            self.announce_scope(context, cx);
        }
        self.send(&Outgoing::Event {
            context: context.into(),
            event,
            session_dir: host.session_dir(cx),
            document: host.active_document(cx),
        });
        if session_changed {
            let views: Vec<_> = self
                .views
                .iter_mut()
                .filter_map(|(id, view)| {
                    (view.context == context).then(|| {
                        view.queue.reset();
                        id.clone()
                    })
                })
                .collect();
            for view in views {
                self.render(&view, cx);
            }
        }
    }

    pub(in crate::ui::extensions::declarative) fn rehost(
        &mut self,
        view: &str,
        workspace: WeakEntity<Workspace>,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let Some(attached) = self.views.get(view) else {
            return;
        };
        let scope = self.scopes.get(&attached.context);
        let mut host = scope
            .map_or(&attached.host, |scope| &scope.host)
            .rehost(workspace);
        if let Some(scope) = scope {
            host.inherit(scope.source.clone());
        }
        let context = host.scope();
        let attached = self.views.get_mut(view).unwrap();
        attached.context = context.clone();
        attached.host = host.clone();
        attached.window = window;
        attached.route_generation = attached.route_generation.wrapping_add(1);
        let generation = attached.route_generation;
        let view = view.to_owned();
        let runtime = cx.entity();
        // Destination Workspace/Chat may themselves still be updating during
        // transfer. Read the new host and retain its process after that turn.
        cx.defer(move |cx| {
            if runtime
                .read(cx)
                .views
                .get(&view)
                .is_none_or(|view| view.route_generation != generation)
            {
                return;
            }
            super::super::rehost_runtime(&runtime, &host, cx);
            runtime.update(cx, |this, cx| {
                this.ensure_scope(host, window, cx);
                this.send(&Outgoing::Rehost { view, context });
                this.prune_scopes();
                cx.notify();
            });
        });
    }

    pub(super) fn prune_scopes(&mut self) {
        let unused: Vec<_> = self
            .scopes
            .keys()
            .filter(|context| !self.views.values().any(|view| &view.context == *context))
            .cloned()
            .collect();
        for context in unused {
            self.scopes.remove(&context);
            self.send(&Outgoing::CloseContext { context });
        }
    }

    pub(super) fn reset_scopes(&mut self) {
        for scope in self.scopes.values_mut() {
            scope.announced = false;
        }
    }

    pub(super) fn content_context(&self, cx: &App) -> serde_json::Value {
        let theme = cx.theme();
        serde_json::json!({"theme": {
            "foreground": theme.foreground.to_hex(), "background": theme.background.to_hex(),
            "muted": theme.muted_foreground.to_hex(), "border": theme.border.to_hex(),
            "accent": theme.primary.to_hex(), "danger": theme.danger.to_hex(),
            "surface": theme.muted.to_hex(), "surface_alt": theme.group_box.to_hex(),
            "font_family": theme.font_family.to_string(), "font_family_mono": theme.mono_font_family.to_string(),
        }})
    }
}
