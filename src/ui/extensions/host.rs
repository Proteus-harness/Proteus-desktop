use std::path::PathBuf;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppSessionActivity;

use crate::api::Api;
use crate::preferences::BadgePlace;
use crate::tabs::ChatKey;
use crate::ui::chat::{Chat, Session};
use crate::ui::motion::PopupPanel;
use crate::ui::workspace::Workspace;

mod scope;
pub(in crate::ui) use scope::{Lifetime, RuntimeScope, ViewOwner};

/// What an extension gets from the client: read access to the active chat's
/// session,
/// the server API, host events and navigation. It holds no strong reference,
/// so extensions never keep the workspace alive.
///
/// Reads must happen outside workspace and chat updates: views read it while
/// rendering or from event handlers, never from their constructors (defer
/// the first load instead).
#[derive(Clone)]
pub(in crate::ui) struct Host {
    workspace: WeakEntity<Workspace>,
    extension: &'static str,
    /// A retained content view belongs to its original chat, even in a background tab.
    bound: Option<(WeakEntity<Chat>, ChatKey)>,
    /// A document-only detached window keeps its originating session until
    /// the destination explicitly chooses a chat. This is weak, never a pump.
    inherited: Option<(WeakEntity<Chat>, ChatKey)>,
    runtime_scope: RuntimeScope,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum HostEvent {
    /// Another chat became active, or the last one closed.
    SessionOpened,
    /// Session summaries arrived; the addressed folder may now be known.
    /// This does not change the session or its transcript projection.
    ContextChanged,
    /// A run of the active chat finished; files and quotas may differ.
    RunSettled,
    NavigationChanged,
}

impl EventEmitter<HostEvent> for Workspace {}

impl Host {
    pub(in crate::ui) fn document_title(&self, argument: &str, title: String, cx: &mut App) {
        if !self.runtime_scope.lifetime.session() {
            return;
        }
        let id = crate::tabs::document(self.extension, argument);
        let _ = self.workspace.update(cx, |workspace, cx| {
            if workspace
                .document_titles
                .get(&id)
                .is_none_or(|current| current.as_ref() != title)
            {
                workspace.document_titles.insert(id, title.into());
                cx.notify();
            }
        });
    }
    pub(in crate::ui) fn retain_process(&self, process: AnyEntity, cx: &mut App) {
        if !self.retains_process() {
            return;
        }
        let _ = self.workspace.update(cx, |workspace, _| {
            workspace
                .extension_processes
                .insert(self.extension, process);
        });
    }
    pub(in crate::ui) fn session_snapshot(
        &self,
        include_entries: bool,
        cx: &App,
    ) -> crate::declarative::protocol::Outgoing {
        let projection = self
            .supports_core(cx)
            .then(|| {
                self.read(cx, |host| {
                    host.session().map(|session| {
                        (
                            Box::new(super::session::state(session, host.activity().cloned())),
                            if include_entries {
                                super::session::entries(
                                    session,
                                    0..session.transcript.entries().len(),
                                )
                            } else {
                                Vec::new()
                            },
                        )
                    })
                })
                .flatten()
            })
            .flatten();
        let (state, entries) = projection.map_or((None, Vec::new()), |(state, entries)| {
            (Some(state), entries)
        });
        crate::declarative::protocol::Outgoing::SessionSnapshot {
            context: self.scope(),
            session_dir: self.session_dir(cx),
            state,
            entries,
        }
    }

    pub(in crate::ui) fn subscribe_session<T: 'static>(
        &self,
        cx: &mut Context<T>,
        mut on_event: impl FnMut(&mut T, &super::session::SessionChanged, &mut Context<T>) + 'static,
    ) -> Option<Subscription> {
        if !self.runtime_scope.lifetime.session() || !self.supports_core(cx) {
            return None;
        }
        let session = self
            .bound
            .as_ref()
            .and_then(|(_, key)| key.proteus_dir().map(std::path::Path::to_path_buf));
        let host = self.clone();
        Some(cx.subscribe(
            &self.workspace.upgrade()?,
            move |this, _, event: &super::session::SessionChanged, cx| {
                if host.supports_core(cx)
                    && session
                        .as_ref()
                        .is_none_or(|session| *session == event.session_dir)
                {
                    on_event(this, event, cx)
                }
            },
        ))
    }
    pub(in crate::ui) fn identity(&self) -> EntityId {
        self.bound
            .as_ref()
            .map_or_else(|| self.workspace.entity_id(), |(chat, _)| chat.entity_id())
    }
    /// Window-scoped protocol context. Bound content has a separate runtime
    /// per chat, while retaining the chat's own session across tab changes.
    pub(in crate::ui) fn scope(&self) -> String {
        self.runtime_scope
            .context()
            .unwrap_or_else(|| self.workspace.entity_id().to_string())
    }

    pub(in crate::ui) fn runtime_scope(&self) -> RuntimeScope {
        self.runtime_scope
    }

    fn retains_process(&self) -> bool {
        self.runtime_scope.lifetime.session() && self.bound.is_none()
    }

    /// Root settings/preview scope. Clones and with_extension preserve this
    /// owner, so every nested renderer stays outside production runtimes.
    pub(in crate::ui) fn isolated(mut self, lifetime: Lifetime) -> Self {
        self.runtime_scope = RuntimeScope::isolated(lifetime);
        self.bound = None;
        self.inherited = None;
        self
    }

    pub(in crate::ui) fn rehost(&self, workspace: WeakEntity<Workspace>) -> Self {
        Self {
            workspace,
            ..self.clone()
        }
    }

    pub(in crate::ui) fn inherit(&mut self, source: Option<(WeakEntity<Chat>, ChatKey)>) {
        if self.bound.is_none() {
            self.inherited = source;
        }
    }

    pub(in crate::ui) fn session_source(&self, cx: &App) -> Option<(WeakEntity<Chat>, ChatKey)> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        if let Some(source) = &self.bound {
            return Some(source.clone());
        }
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);
        if let (Some(chat), Some(key)) = (workspace.active_chat_view(), workspace.active_chat_key())
        {
            return Some((chat.downgrade(), key));
        }
        self.inherited.clone()
    }
    pub(in crate::ui) fn new(workspace: WeakEntity<Workspace>, extension: &'static str) -> Self {
        Self {
            workspace,
            extension,
            bound: None,
            inherited: None,
            runtime_scope: RuntimeScope {
                content_only: super::find(extension)
                    .is_some_and(|manifest| !manifest.requires_core),
                ..RuntimeScope::default()
            },
        }
    }

    pub(in crate::ui) fn for_chat(mut self, chat: WeakEntity<Chat>, key: ChatKey) -> Self {
        self.runtime_scope.content_only |= key.proteus_dir().is_none();
        self.bound = Some((chat, key));
        self
    }

    /// Id of the extension this host was handed to.
    pub(in crate::ui) fn extension(&self) -> &'static str {
        self.extension
    }

    pub(in crate::ui) fn with_extension(&self, extension: &'static str) -> Self {
        let mut runtime_scope = self.runtime_scope;
        runtime_scope.content_only |=
            super::find(extension).is_some_and(|manifest| !manifest.requires_core);
        Self {
            extension,
            runtime_scope,
            ..self.clone()
        }
    }

    /// Folder the agent works in.
    pub(in crate::ui) fn workspace_path(&self, cx: &App) -> Option<PathBuf> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);
        let key = self
            .bound
            .as_ref()
            .map(|(_, key)| key.clone())
            .or_else(|| workspace.active_chat_key())
            .or_else(|| self.inherited.as_ref().map(|(_, key)| key.clone()));
        Some(workspace.project_for_key(key.as_ref()).to_path_buf())
    }

    pub(in crate::ui) fn read<R>(&self, cx: &App, read: impl FnOnce(&HostView) -> R) -> Option<R> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        let workspace = self.workspace.upgrade()?.read(cx);
        let bound_chat = self.bound.as_ref().and_then(|(chat, _)| chat.upgrade());
        let inherited_chat = self.inherited.as_ref().and_then(|(chat, _)| chat.upgrade());
        let chat = if self.bound.is_some() {
            bound_chat.as_ref().map(|chat| chat.read(cx))
        } else {
            workspace
                .active_chat_view()
                .or(inherited_chat.as_ref())
                .map(|chat| chat.read(cx))
        };
        Some(read(&HostView { workspace, chat }))
    }

    /// The actual framed panel behind a badge, shared by every process popup.
    pub(in crate::ui) fn badge_panel(&self, place: BadgePlace, cx: &App) -> Option<PopupPanel> {
        use gpui_kit::base::{Align, Placement};
        let theme = cx.theme();
        self.read(cx, |host| {
            let (bounds, placement, background) = match place {
                BadgePlace::ComposerStart | BadgePlace::ComposerEnd => (
                    host.chat?.composer_strip_bounds(),
                    Placement::Top,
                    theme.group_box,
                ),
                BadgePlace::HeaderStart | BadgePlace::HeaderEnd => (
                    host.workspace.header_bounds.get(),
                    Placement::Bottom,
                    theme.title_bar,
                ),
                BadgePlace::RailStart | BadgePlace::RailEnd => (
                    host.workspace.rail_bounds.get(),
                    Placement::Right,
                    theme.status_bar,
                ),
                BadgePlace::Hidden => return None,
            };
            let align = match place {
                BadgePlace::ComposerEnd | BadgePlace::HeaderEnd | BadgePlace::RailEnd => Align::End,
                _ => Align::Start,
            };
            (bounds.size.width > px(0.) && bounds.size.height > px(0.)).then_some(PopupPanel {
                bounds,
                placement,
                align,
                background,
            })
        })
        .flatten()
    }

    /// The active chat, for extensions that change its session settings.
    pub(in crate::ui) fn chat(&self, cx: &App) -> Option<Entity<Chat>> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        if let Some((chat, _)) = &self.bound {
            return chat.upgrade();
        }
        let workspace = self.workspace.upgrade()?;
        workspace
            .read(cx)
            .active_chat_view()
            .cloned()
            .or_else(|| self.inherited.as_ref()?.0.upgrade())
    }

    /// Core capability is attached to the addressed conversation, not the
    /// existence of a Core process elsewhere in this window group.
    pub(in crate::ui) fn supports_core(&self, cx: &App) -> bool {
        self.session_dir(cx).is_some()
    }

    pub(in crate::ui) fn api(&self, cx: &App) -> Option<Api> {
        if !self.supports_core(cx) {
            return None;
        }
        self.workspace.upgrade()?.read(cx).api()
    }

    /// Immutable host identity is safe to inspect while its Chat is rendering.
    /// Do not route metadata reads through Host::read: that leases the Chat again.
    pub(in crate::ui) fn session_dir(&self, cx: &App) -> Option<PathBuf> {
        self.context_session(|| self.workspace.upgrade()?.read(cx).active_chat_key())
    }

    fn context_session(&self, active: impl FnOnce() -> Option<ChatKey>) -> Option<PathBuf> {
        if !self.runtime_scope.lifetime.session() || self.runtime_scope.content_only {
            return None;
        }
        if let Some((_, key)) = &self.bound {
            return key
                .proteus_dir()
                .map(std::path::Path::to_path_buf)
                .and_then(nonempty_session);
        }
        match active() {
            Some(key) => key
                .proteus_dir()
                .map(std::path::Path::to_path_buf)
                .and_then(nonempty_session),
            None => self
                .inherited
                .as_ref()
                .and_then(|(_, key)| key.proteus_dir())
                .map(std::path::Path::to_path_buf)
                .and_then(nonempty_session),
        }
    }

    pub(in crate::ui) fn active_document(&self, cx: &App) -> Option<String> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        let workspace = self.workspace.upgrade()?;
        let layout = workspace.read(cx).tab_layout(cx);
        let (owner, argument) = crate::tabs::split_document(layout.active(layout.focused)?)?;
        (owner == self.extension).then(|| argument.to_string())
    }

    /// Calls `on_event` for every host event while `cx`'s entity lives.
    pub(in crate::ui) fn subscribe<T: 'static>(
        &self,
        cx: &mut Context<T>,
        mut on_event: impl FnMut(&mut T, HostEvent, &mut Context<T>) + 'static,
    ) -> Option<Subscription> {
        // A content source is immutable and bound to its chat. Switching the
        // active tab must neither repaint it nor change its session context.
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        let bound = self.bound.is_some();
        let workspace = self.workspace.upgrade()?;
        Some(
            cx.subscribe(&workspace, move |this, _, event: &HostEvent, cx| {
                if !bound || *event == HostEvent::ContextChanged {
                    on_event(this, *event, cx)
                }
            }),
        )
    }

    /// Opens a document of extension `extension` as a tab and returns its
    /// view. A preview tab is replaced by the next preview until it is kept.
    pub(in crate::ui) fn open_document(
        &self,
        extension: &'static str,
        argument: &str,
        open: OpenDocument,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        let id = crate::tabs::document(extension, argument);
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.open_document(&id, open, window, cx)
            })
            .ok()
            .flatten()
    }

    /// Brings an extension forward: opens or shows its panel.
    pub(in crate::ui) fn activate(&self, id: &'static str, window: &mut Window, cx: &mut App) {
        if !self.runtime_scope.lifetime.session() {
            return;
        }
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.activate_extension(id, window, cx)
            })
            .ok();
    }
}

fn nonempty_session(session: PathBuf) -> Option<PathBuf> {
    (!session.as_os_str().is_empty()).then_some(session)
}

#[cfg(test)]
mod isolated_tests {
    use super::{Host, Lifetime};
    use crate::tabs::ChatKey;
    use gpui_kit::WeakEntity;

    #[test]
    fn local_manifest_scope_cannot_be_widened_by_binding_a_core_chat() {
        let mut host = Host::new(WeakEntity::new_invalid(), "local-test");
        // The same scope set by Host::new from dependencies.core=false.
        host.runtime_scope.content_only = true;
        let bound = host.for_chat(
            WeakEntity::new_invalid(),
            ChatKey::Proteus("/sessions/core".into()),
        );
        assert!(bound.runtime_scope().content_only);
        assert_eq!(
            bound.context_session(|| panic!("local host read active chat")),
            None
        );
        let child = bound.with_extension("core-required-child");
        assert_eq!(
            child.context_session(|| panic!("child widened local host")),
            None
        );
        assert!(child.runtime_scope().content_only);
    }

    #[test]
    fn external_bound_context_cannot_fall_back_to_a_core_directory() {
        let host = Host::new(WeakEntity::new_invalid(), "renderer").for_chat(
            WeakEntity::new_invalid(),
            ChatKey::Codex("opaque:codex".into()),
        );
        assert!(host.runtime_scope().content_only);
        assert_eq!(
            host.context_session(|| panic!("bound Codex metadata read workspace/chat")),
            None
        );
        let (server, _) = host.runtime_scope().connection(
            || panic!("Codex content read Core credentials"),
            || Some("/project".into()),
        );
        let server = server.unwrap();
        assert!(server.token.is_empty());
        assert!(server.origin.is_empty());
    }

    #[test]
    fn retained_bound_and_inherited_directories_survive_the_chat_entity() {
        let mut inherited = Host::new(WeakEntity::new_invalid(), "files");
        inherited.inherit(Some((
            WeakEntity::new_invalid(),
            ChatKey::Proteus("/sessions/b".into()),
        )));
        assert_eq!(
            inherited.context_session(|| None),
            Some("/sessions/b".into())
        );
        assert_eq!(
            inherited.context_session(|| Some(ChatKey::Proteus("/sessions/a".into()))),
            Some("/sessions/a".into())
        );

        assert_eq!(
            inherited.context_session(|| Some(ChatKey::Codex("active:external".into()))),
            None,
            "active external driver must suppress an inherited Core directory"
        );

        let bound = inherited.for_chat(
            WeakEntity::new_invalid(),
            ChatKey::Proteus("/sessions/bound".into()),
        );
        assert_eq!(
            bound.context_session(|| panic!("bound Core metadata read workspace/chat")),
            Some("/sessions/bound".into())
        );
        assert_eq!(
            bound.context_session(|| Some(ChatKey::Proteus("/sessions/a".into()))),
            Some("/sessions/bound".into())
        );
    }

    #[test]
    fn content_inline_and_rehost_clones_cannot_escape_the_view_owner() {
        for lifetime in [Lifetime::Settings, Lifetime::Preview] {
            let production = Host::new(WeakEntity::new_invalid(), "renderer");
            assert!(production.retains_process());
            let parent = production.clone().isolated(lifetime);
            let child = parent.with_extension("nested-renderer");
            let inline = child
                .clone()
                .with_extension("inline-renderer")
                .rehost(WeakEntity::new_invalid());
            for host in [&parent, &child, &inline] {
                assert_eq!(host.runtime_scope(), parent.runtime_scope());
                assert_eq!(host.scope(), parent.scope());
                assert!(
                    !host.retains_process(),
                    "nested runtime entered Workspace.extension_processes"
                );
                assert!(host.runtime_scope().lifetime.allowed(false));
                host.runtime_scope().connection(
                    || panic!("nested host accessed production server"),
                    || panic!("nested host accessed production folder"),
                );
            }
            assert_ne!(
                parent.runtime_scope().owner,
                production.isolated(lifetime).runtime_scope().owner
            );
        }
    }
}

/// How [`Host::open_document`] shows a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) struct OpenDocument {
    /// Temporary tab, replaced by the next preview.
    pub preview: bool,
    /// Move keyboard focus to the document.
    pub focus: bool,
}

/// Read-only view of the workspace for one call of [`Host::read`].
pub(in crate::ui) struct HostView<'a> {
    workspace: &'a Workspace,
    chat: Option<&'a Chat>,
}

impl HostView<'_> {
    /// Session of the active chat.
    pub(in crate::ui) fn session(&self) -> Option<&Session> {
        self.chat
            .filter(|chat| chat.supports_core())
            .map(|chat| &chat.session)
    }

    /// Server-side activity of the active chat's session.
    pub(in crate::ui) fn activity(&self) -> Option<&AppSessionActivity> {
        let session = self.session()?;
        self.workspace
            .sessions
            .iter()
            .find(|summary| summary.key == session.key)?
            .activity
            .as_ref()
    }
}
