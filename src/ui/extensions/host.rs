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
    /// A detached document can read its own root chat from another window.
    /// Dynamic panels never use this as a fallback when no space is selected.
    inherited: Option<(WeakEntity<Chat>, ChatKey)>,
    /// A retained document keeps its root while another space is displayed.
    space: Option<ChatKey>,
    /// Composer controls follow the currently addressed chat, including a child.
    command_chat: bool,
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
            if let Some(root) = &self.space {
                workspace.set_document_title_for_space(root, &id, title, cx);
                return;
            }
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
        let host = self.clone();
        Some(cx.subscribe(
            &self.workspace.upgrade()?,
            move |this, _, event: &super::session::SessionChanged, cx| {
                if host.session_dir(cx).as_ref() == Some(&event.session_dir) {
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
    /// Retained documents use a stable context inside the shared process;
    /// panels follow the displayed space, composer controls the command chat,
    /// and bound content its own chat.
    pub(in crate::ui) fn scope(&self) -> String {
        self.runtime_scope.context().unwrap_or_else(|| {
            let context =
                space_context(self.workspace.entity_id().to_string(), self.space.as_ref());
            if self.command_chat {
                format!("{context}:command")
            } else {
                context
            }
        })
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
        self.space = None;
        self.command_chat = false;
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
            self.inherited =
                source.filter(|(_, key)| self.space.as_ref().is_none_or(|root| root == key));
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
        if let Some(root) = self.context_key(|| {
            self.dynamic_key(
                || workspace.space_root_key(),
                || workspace.active_chat_key(),
            )
        }) {
            return workspace
                .chats
                .get(&root.tab_id())
                .map(|chat| (chat.view.downgrade(), root.clone()))
                .or_else(|| self.inherited_root(&root).cloned());
        }
        None
    }

    pub(in crate::ui) fn new(workspace: WeakEntity<Workspace>, extension: &'static str) -> Self {
        Self {
            workspace,
            extension,
            bound: None,
            inherited: None,
            space: None,
            command_chat: false,
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
        self.space = None;
        self.command_chat = false;
        self
    }

    /// Saved conversations can render content without handing any renderer
    /// credentials or a command capability for the foreign session.
    pub(in crate::ui) fn content_only(mut self) -> Self {
        self.runtime_scope.content_only = true;
        self
    }

    /// Pin a document to its root without creating another extension process.
    /// An isolated/local host retains its narrower runtime capabilities.
    pub(in crate::ui) fn for_space(mut self, root: ChatKey) -> Self {
        self.runtime_scope.content_only |= root.proteus_dir().is_none();
        self.space = Some(root);
        self.bound = None;
        self.inherited = None;
        self.command_chat = false;
        self
    }

    /// Composer badges address the same dynamic chat as the input, without
    /// allocating another process or borrowing a currently rendering Chat.
    pub(in crate::ui) fn for_command_chat(mut self) -> Self {
        self.command_chat = true;
        self.bound = None;
        self.space = None;
        self.inherited = None;
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
        let key = self.context_key(|| {
            self.dynamic_key(
                || workspace.space_root_key(),
                || workspace.active_chat_key(),
            )
        });
        if self.space.is_some()
            && !key.as_ref().is_some_and(|key| {
                key.proteus_dir()
                    .is_some_and(|dir| workspace.session_projects.contains_key(dir))
                    || workspace.sessions.iter().any(|summary| &summary.key == key)
            })
        {
            // Late metadata can fill this in; another displayed folder cannot.
            return None;
        }
        Some(workspace.project_for_key(key.as_ref()).to_path_buf())
    }

    pub(in crate::ui) fn read<R>(&self, cx: &App, read: impl FnOnce(&HostView) -> R) -> Option<R> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        let chat = self.chat(cx);
        let workspace = self.workspace.upgrade()?.read(cx);
        let chat = chat.as_ref().map(|chat| chat.read(cx));
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
        let workspace = workspace.read(cx);
        if let Some(root) = &self.space {
            return workspace
                .chats
                .get(&root.tab_id())
                .map(|chat| chat.view.clone())
                .or_else(|| self.inherited_root(root)?.0.upgrade());
        }
        if self.command_chat {
            return workspace.active_chat_view().cloned();
        }
        if workspace.space_root_key().is_some() {
            return workspace.space_root_chat_view().cloned();
        }
        None
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
        self.context_session(|| {
            let workspace = self.workspace.upgrade()?;
            let workspace = workspace.read(cx);
            self.dynamic_key(
                || workspace.space_root_key(),
                || workspace.active_chat_key(),
            )
        })
    }

    fn context_session(&self, active: impl FnOnce() -> Option<ChatKey>) -> Option<PathBuf> {
        if !self.runtime_scope.lifetime.session() || self.runtime_scope.content_only {
            return None;
        }
        self.context_key(active)?
            .proteus_dir()
            .map(std::path::Path::to_path_buf)
            .and_then(nonempty_session)
    }

    fn context_key(&self, active: impl FnOnce() -> Option<ChatKey>) -> Option<ChatKey> {
        self.bound
            .as_ref()
            .map(|(_, key)| key.clone())
            .or_else(|| self.space.clone())
            .or_else(active)
    }

    /// Keep selection lazy: querying a bound/local host never reads another
    /// chat, and an empty command target never falls back to the area's root.
    fn dynamic_key(
        &self,
        root: impl FnOnce() -> Option<ChatKey>,
        command: impl FnOnce() -> Option<ChatKey>,
    ) -> Option<ChatKey> {
        if self.command_chat { command() } else { root() }
    }

    fn inherited_root(&self, root: &ChatKey) -> Option<&(WeakEntity<Chat>, ChatKey)> {
        self.space
            .as_ref()
            .filter(|fixed| *fixed == root)
            .and_then(|_| self.inherited.as_ref())
            .filter(|(_, key)| key == root)
    }

    fn displays_space(&self, workspace: &Workspace) -> bool {
        self.space
            .as_ref()
            .is_none_or(|root| workspace.space_root_key().as_ref() == Some(root))
    }

    pub(in crate::ui) fn active_document(&self, cx: &App) -> Option<String> {
        if !self.runtime_scope.lifetime.session() {
            return None;
        }
        let workspace = self.workspace.upgrade()?;
        let workspace = workspace.read(cx);
        if !self.displays_space(workspace) {
            return None;
        }
        let layout = workspace.tab_layout(cx);
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
                self.displays_space(workspace)
                    .then(|| workspace.open_document(&id, open, window, cx))
                    .flatten()
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
                if self.displays_space(workspace) {
                    workspace.activate_extension(id, window, cx)
                }
            })
            .ok();
    }
}

fn space_context(workspace: String, root: Option<&ChatKey>) -> String {
    match root {
        Some(root) => format!("{workspace}:space:{}", root.tab_id()),
        None => workspace,
    }
}

fn nonempty_session(session: PathBuf) -> Option<PathBuf> {
    (!session.as_os_str().is_empty()).then_some(session)
}

#[cfg(test)]
mod tests;

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
