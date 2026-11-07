//! Window routes of a shared process. A move changes only one attachment;
//! subscription lifetimes follow the windows that still own its surfaces.

use super::*;
use crate::ui::chat::Chat;
use crate::ui::workspace::Workspace;
use gpui_kit::component::*;

/// Refresh an existing scope's folder without a session lifecycle transition.
/// NavigationChanged makes existing protocol-4 extensions redraw after Context;
/// Context alone only updates the SDK's metadata.
fn workspace_update(
    context: &str,
    current: &std::path::Path,
    workspace: &std::path::Path,
    session_dir: Option<PathBuf>,
    document: Option<String>,
) -> Option<[Outgoing; 2]> {
    (current != workspace).then(|| {
        [
            Outgoing::Context {
                context: context.into(),
                workspace: workspace.into(),
                session_dir: session_dir.clone(),
            },
            Outgoing::Event {
                context: context.into(),
                event: HostEventKind::NavigationChanged,
                session_dir,
                document,
            },
        ]
    })
}

/// GPUI preserves the originating Chat event's FIFO order. A shared runtime
/// forwards it once even when several workspace-local subscriptions receive it.
#[derive(Default)]
pub(super) struct EventCursor(u64);
impl EventCursor {
    fn admit(&mut self, id: u64) -> bool {
        if id <= self.0 {
            return false;
        }
        self.0 = id;
        true
    }
}

pub(super) struct Scope {
    pub(super) host: Host,
    pub(super) window: AnyWindowHandle,
    pub(super) popup_closed: u64,
    source: Option<(WeakEntity<Chat>, crate::tabs::ChatKey)>,
    session: Option<PathBuf>,
    workspace: PathBuf,
    announced: bool,
    _events: Option<Subscription>,
    _session_events: Option<Subscription>,
}

#[cfg(test)]
mod tests {
    use super::workspace_update;
    use std::path::{Path, PathBuf};

    #[test]
    fn originating_event_is_forwarded_once_without_electing_a_destination_subscription() {
        let mut cursor = super::EventCursor::default();
        // Only the source Workspace receives the shared Chat's event. A
        // document-only destination has no subscription to that emitter.
        assert!(cursor.admit(10));
        // A second surface/source copy must not splice that event twice.
        assert!(!cursor.admit(10));
        // Identical payload in a different event remains a real update.
        assert!(cursor.admit(11));
        assert!(!cursor.admit(11));
    }

    #[test]
    fn restored_chat_gets_its_late_project_without_a_second_session_open() {
        let session = Some(PathBuf::from("/sessions/chat-b"));
        let messages = workspace_update(
            "window",
            Path::new("/project-a"),
            Path::new("/project-b"),
            session,
            Some("README.md".into()),
        )
        .unwrap();
        let messages: Vec<_> = messages
            .iter()
            .map(|message| serde_json::to_value(message).unwrap())
            .collect();
        assert_eq!(
            messages,
            vec![
                serde_json::json!({"type":"context", "context":"window", "workspace":"/project-b", "session_dir":"/sessions/chat-b"}),
                serde_json::json!({"type":"event", "context":"window", "event":"navigation_changed", "session_dir":"/sessions/chat-b", "document":"README.md"}),
            ]
        );
        // Repeated status/catalog refreshes leave retained extension views alone.
        assert!(
            workspace_update(
                "window",
                Path::new("/project-b"),
                Path::new("/project-b"),
                Some("/sessions/chat-b".into()),
                None
            )
            .is_none()
        );
    }
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
        let events = self
            .lifetime
            .session()
            .then(|| {
                host.subscribe(cx, move |this, event, cx| {
                    this.scope_event(&events_context, event, cx)
                })
            })
            .flatten();
        let session_context = context.clone();
        let session_events = self
            .lifetime
            .session()
            .then(|| {
                host.subscribe_session(cx, move |this: &mut Self, event, _| {
                    // Workspace subscriptions are local; never elect a
                    // context which may not receive this source event.
                    if this
                        .extension
                        .manifest
                        .accepts_session_update(&event.update)
                        && this.session_events.admit(event.event_id)
                    {
                        this.send(&Outgoing::SessionUpdate {
                            context: session_context.clone(),
                            session_dir: event.session_dir.clone(),
                            update: event.update.clone(),
                        });
                    }
                })
            })
            .flatten();
        let source = self
            .lifetime
            .session()
            .then(|| host.session_source(cx))
            .flatten();
        self.scopes.insert(
            context.clone(),
            Scope {
                host,
                window,
                popup_closed: 0,
                source,
                session: None,
                workspace: PathBuf::new(),
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
        let session = self
            .lifetime
            .session()
            .then(|| host.session_dir(cx))
            .flatten();
        let snapshot_known = self
            .scopes
            .iter()
            .any(|(id, scope)| id != context && scope.announced && scope.session == session);
        let workspace = if !self.lifetime.session() {
            PathBuf::new()
        } else {
            host.workspace_path(cx).unwrap_or_default()
        };
        self.send(&Outgoing::Context {
            context: context.into(),
            workspace: workspace.clone(),
            session_dir: session.clone(),
        });
        if self.lifetime.session() && self.extension.manifest.session_events && !snapshot_known {
            self.send(&host.session_snapshot(self.extension.manifest.session_entries, cx));
        }
        if let Some(scope) = self.scopes.get_mut(context) {
            scope.session = session;
            scope.workspace = workspace;
            scope.announced = true;
        }
    }

    fn scope_event(&mut self, context: &str, event: HostEvent, cx: &mut Context<Self>) {
        let Some(scope) = self.scopes.get_mut(context) else {
            return;
        };
        if event == HostEvent::ContextChanged {
            // A late catalog changes folder metadata only. Do not announce
            // SessionOpened or resend a potentially large session snapshot.
            if !scope.announced || self.process.is_none() {
                return;
            }
            let workspace = scope.host.workspace_path(cx).unwrap_or_default();
            let messages = workspace_update(
                context,
                &scope.workspace,
                &workspace,
                scope.host.session_dir(cx),
                scope.host.active_document(cx),
            );
            scope.workspace = workspace;
            if let Some(messages) = messages {
                for message in messages {
                    self.send(&message);
                }
            }
            return;
        }
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
            HostEvent::ContextChanged => unreachable!("metadata refresh handled above"),
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
            if runtime.read(cx).lifetime == Lifetime::Workspace {
                super::super::rehost_runtime(&runtime, &host, cx);
            }
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
            "radius": f32::from(theme.radius), "radius_lg": f32::from(theme.radius_lg),
        }})
    }
}
