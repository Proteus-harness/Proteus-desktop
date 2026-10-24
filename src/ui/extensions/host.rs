use std::path::PathBuf;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppSessionActivity;

use crate::api::Api;
use crate::preferences::BadgePlace;
use crate::ui::chat::{Chat, Session};
use crate::ui::motion::PopupPanel;
use crate::ui::workspace::Workspace;

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
    bound: Option<(WeakEntity<Chat>, PathBuf)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum HostEvent {
    /// Another chat became active, or the last one closed.
    SessionOpened,
    /// A run of the active chat finished; files and quotas may differ.
    RunSettled,
    NavigationChanged,
}

impl EventEmitter<HostEvent> for Workspace {}

impl Host {
    pub(in crate::ui) fn document_title(&self, argument: &str, title: String, cx: &mut App) {
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
        if self.bound.is_some() {
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
        cx: &App,
    ) -> crate::declarative::protocol::Outgoing {
        let (state, entries) = self
            .read(cx, |host| {
                host.session().map(|session| {
                    (
                        Box::new(super::session::state(session, host.activity().cloned())),
                        super::session::entries(session, 0..session.transcript.entries().len()),
                    )
                })
            })
            .flatten()
            .map_or((None, Vec::new()), |(state, entries)| {
                (Some(state), entries)
            });
        crate::declarative::protocol::Outgoing::SessionSnapshot {
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
        let session = self.bound.as_ref().map(|(_, session)| session.clone());
        Some(cx.subscribe(
            &self.workspace.upgrade()?,
            move |this, _, event: &super::session::SessionChanged, cx| {
                if session
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
    pub(in crate::ui) fn new(workspace: WeakEntity<Workspace>, extension: &'static str) -> Self {
        Self {
            workspace,
            extension,
            bound: None,
        }
    }

    pub(in crate::ui) fn for_chat(mut self, chat: WeakEntity<Chat>, session: PathBuf) -> Self {
        self.bound = Some((chat, session));
        self
    }

    /// Id of the extension this host was handed to.
    pub(in crate::ui) fn extension(&self) -> &'static str {
        self.extension
    }

    /// Folder the agent works in.
    pub(in crate::ui) fn workspace_path(&self, cx: &App) -> Option<PathBuf> {
        self.read(cx, |host| host.workspace.workspace_path.clone())
    }

    pub(in crate::ui) fn read<R>(&self, cx: &App, read: impl FnOnce(&HostView) -> R) -> Option<R> {
        let workspace = self.workspace.upgrade()?.read(cx);
        let bound_chat = self.bound.as_ref().and_then(|(chat, _)| chat.upgrade());
        let chat = if self.bound.is_some() {
            bound_chat.as_ref().map(|chat| chat.read(cx))
        } else {
            workspace.active_chat_view().map(|chat| chat.read(cx))
        };
        Some(read(&HostView { workspace, chat }))
    }

    /// The actual framed panel behind a badge, shared by every process popup.
    pub(in crate::ui) fn badge_panel(&self, place: BadgePlace, cx: &App) -> Option<PopupPanel> {
        use gpui_kit::base::Placement;
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
            (bounds.size.width > px(0.) && bounds.size.height > px(0.)).then_some(PopupPanel {
                bounds,
                placement,
                background,
            })
        })
        .flatten()
    }

    /// The active chat, for extensions that change its session settings.
    pub(in crate::ui) fn chat(&self, cx: &App) -> Option<Entity<Chat>> {
        if let Some((chat, _)) = &self.bound {
            return chat.upgrade();
        }
        let workspace = self.workspace.upgrade()?;
        workspace.read(cx).active_chat_view().cloned()
    }

    pub(in crate::ui) fn api(&self, cx: &App) -> Option<Api> {
        self.read(cx, |host| host.workspace.api()).flatten()
    }

    pub(in crate::ui) fn session_dir(&self, cx: &App) -> Option<PathBuf> {
        if let Some((_, session)) = &self.bound {
            return Some(session.clone());
        }
        self.read(cx, |host| host.session().map(|session| session.dir.clone()))
            .flatten()
    }

    pub(in crate::ui) fn active_document(&self, cx: &App) -> Option<String> {
        self.read(cx, |host| host.active_document(self.extension, cx))
            .flatten()
    }

    /// Calls `on_event` for every host event while `cx`'s entity lives.
    pub(in crate::ui) fn subscribe<T: 'static>(
        &self,
        cx: &mut Context<T>,
        mut on_event: impl FnMut(&mut T, HostEvent, &mut Context<T>) + 'static,
    ) -> Option<Subscription> {
        // A content source is immutable and bound to its chat. Switching the
        // active tab must neither repaint it nor change its session context.
        if self.bound.is_some() {
            return None;
        }
        let workspace = self.workspace.upgrade()?;
        Some(
            cx.subscribe(&workspace, move |this, _, event: &HostEvent, cx| {
                on_event(this, *event, cx)
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
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.activate_extension(id, window, cx)
            })
            .ok();
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
        self.chat.map(|chat| &chat.session)
    }

    /// Server-side activity of the active chat's session.
    pub(in crate::ui) fn activity(&self) -> Option<&AppSessionActivity> {
        let session = self.session()?;
        self.workspace
            .sessions
            .iter()
            .find(|summary| summary.session_dir == session.dir)?
            .activity
            .as_ref()
    }

    /// Argument of the document tab active in the focused pane, when the
    /// tab belongs to `extension`.
    pub(in crate::ui) fn active_document(&self, extension: &str, cx: &App) -> Option<String> {
        let layout = self.workspace.tab_layout(cx);
        let (owner, argument) = crate::tabs::split_document(layout.active(layout.focused)?)?;
        (owner == extension).then(|| argument.to_string())
    }
}
