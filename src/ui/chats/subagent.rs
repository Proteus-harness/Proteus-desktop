//! Saved child chats use document tabs, so focusing one cannot address commands
//! to a foreign, unresumed session. Only the parent owns the workspace runtime.
use std::rc::Rc;

use gpui_kit::*;
use proteus_contracts::app_protocol::AppSubagentSession;

use super::super::chat::{Chat, readonly::Target};
use super::super::extensions::SurfaceView;
use super::super::workspace::Workspace;
use crate::tabs::{ChatKey, Edge, PaneId, TabLayout};

fn catalog_target(
    summaries: &[crate::conversations::ConversationSummary],
    key: &ChatKey,
    owner: String,
) -> Option<Target> {
    let summary = summaries.iter().find(|summary| &summary.key == key)?;
    let ChatKey::Proteus(dir) = key else {
        return None;
    };
    Some(Target {
        owner,
        parent: summary.parent.as_ref()?.tab_id(),
        child_thread_id: None,
        session: AppSubagentSession {
            session_id: summary.proteus_session_id?,
            session_dir: dir.clone(),
        },
    })
}

fn open_right_document(layout: &mut TabLayout, id: &str, parent: PaneId) {
    // split moves an existing tab. Add it to the parent's pane before moving
    // it right, all within one layout update so no intermediate frame paints.
    layout.open_in(id, parent, |_| false);
    layout.keep(id);
    layout.split(id, parent, Edge::Right);
}

impl Workspace {
    /// Sidebar/project navigation uses the same saved document as a task link.
    /// Catalog metadata never resumes the child's writer just to view its history.
    pub(super) fn open_catalog_subagent(
        &mut self,
        key: &ChatKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(target) = catalog_target(&self.sessions, key, self.chat_space_root(key)) else {
            return false;
        };
        if !self.begin_navigation(cx) {
            return true;
        }
        if target.owner == key.tab_id() {
            self.report(
                "Не удалось определить родительский чат субагента".into(),
                cx,
            );
            return true;
        }
        self.select_space_root(&target.owner, cx);
        if let Some(root) = crate::tabs::chat_key(&target.owner)
            && !self.chats.contains_key(&target.owner)
        {
            self.ensure_chat(root, window, cx);
        }
        self.open_subagent_target(target, window, cx);
        true
    }

    pub(in crate::ui) fn defer_open_subagent(
        &self,
        owner: String,
        parent: String,
        child_thread_id: String,
        session: AppSubagentSession,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.weak_entity();
        let window = self.window_handle;
        let navigation = self.navigation_generation;
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    workspace
                        .update(cx, |this, cx| {
                            if this.navigation_generation == navigation {
                                this.open_subagent(
                                    owner,
                                    parent,
                                    child_thread_id,
                                    session,
                                    window,
                                    cx,
                                );
                            }
                        })
                        .ok();
                })
                .ok();
        });
    }

    fn open_subagent(
        &mut self,
        owner: String,
        parent: String,
        child_thread_id: String,
        session: AppSubagentSession,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A delayed click cannot change workspaces after the user switched.
        if self.spaces.active_root() != Some(owner.as_str()) || !session.session_dir.is_absolute() {
            return;
        }
        let target = Target {
            owner,
            parent,
            child_thread_id: Some(child_thread_id),
            session,
        };
        self.open_subagent_target(target, window, cx);
    }

    fn open_subagent_target(
        &mut self,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.begin_navigation(cx) {
            return;
        }
        // A task link and the catalog carry different optional task metadata,
        // but address one exact durable session. Reuse even an unloaded tab.
        let id = self
            .layout
            .panes()
            .into_iter()
            .flat_map(|pane| pane.tabs.iter())
            .find_map(|id| {
                Target::parse(id)
                    .filter(|shown| shown.owner == target.owner && shown.session == target.session)
                    .map(|_| id.to_owned())
            })
            .unwrap_or_else(|| target.id());
        if self.show_external_tab(&id, cx) {
            return;
        }
        self.retry_subagent_document(&id, cx);
        if self.layout.contains(&id) {
            self.show_tab_in(&id, None, window, cx);
            return;
        }
        // Reuse the existing right-hand pane; a first child creates it beside
        // the parent instead of covering the conversation with another card.
        let parent_pane = self
            .layout
            .pane_of(&target.owner)
            .unwrap_or(self.layout.focused);
        if let Some(pane) = self.layout.neighbor(parent_pane, Edge::Right) {
            self.show_tab_in(&id, Some(pane), window, cx);
        } else {
            self.update_tabs(cx, |layout| open_right_document(layout, &id, parent_pane));
            self.focus_active(window, cx);
        }
    }

    pub(in crate::ui) fn retry_subagent_document(&self, id: &str, cx: &mut Context<Self>) {
        if let Some(surface) = self.documents.get(id)
            && let Ok(chat) = surface.view.clone().downcast::<Chat>()
        {
            chat.update(cx, |chat, cx| chat.readonly_reopened(cx));
        }
    }

    pub(in crate::ui) fn ensure_subagent_document(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<SurfaceView> {
        let target = Target::parse(id)?;
        if self.spaces.active_root() != Some(target.owner.as_str()) {
            return None;
        }
        let source = self.subagent_source(&target, cx);
        let child = self.subagent_thread(&target, cx);
        if let Some(surface) = self.documents.get(id) {
            if let Ok(chat) = surface.view.clone().downcast::<Chat>() {
                chat.update(cx, |chat, cx| {
                    chat.readonly_thread_bound(child);
                    chat.readonly_source_changed(source, false, true, cx)
                });
            }
            return Some(surface.clone());
        }
        let api = self.api()?;
        let owner = cx.weak_entity();
        let extras = self.composer_extras(cx);
        let chat = cx.new(|cx| {
            let mut chat = Chat::saved_subagent(api, owner, target, source, extras, window, cx);
            chat.readonly_thread_bound(child);
            chat
        });
        let rehost = chat.clone();
        let surface = SurfaceView {
            view: chat.clone().into(),
            focus: chat.focus_handle(cx),
            rehost: Rc::new(move |workspace, window, cx| {
                rehost.update(cx, |chat, cx| {
                    chat.rehost(workspace, Rc::new(|_, _, _| None), window, cx)
                });
            }),
        };
        self.documents.insert(id.to_owned(), surface.clone());
        Some(surface)
    }

    pub(in crate::ui) fn subagent_document_visible(&self, id: &str, entity: EntityId) -> bool {
        self.settings.is_none()
            && self.layout.visible().any(|visible| visible == id)
            && self
                .documents
                .get(id)
                .is_some_and(|surface| surface.view.entity_id() == entity)
    }

    pub(super) fn refresh_subagent_documents(
        &self,
        parent: &str,
        force: bool,
        cx: &mut Context<Self>,
    ) {
        for (id, surface) in self.retained_documents() {
            let Ok(view) = surface.view.clone().downcast::<Chat>() else {
                continue;
            };
            let Some(target) = view.read(cx).readonly_target() else {
                continue;
            };
            if target.parent != parent && !(force && target.owner == parent) {
                continue;
            }
            let source = self.subagent_source(target, cx);
            let child = self.subagent_thread(target, cx);
            let visible =
                self.settings.is_none() && self.layout.visible().any(|visible| visible == id);
            view.update(cx, |chat, cx| {
                chat.readonly_thread_bound(child);
                chat.readonly_source_changed(source, force, visible, cx)
            });
        }
    }

    pub(super) fn refresh_subagent_lifecycle(
        &self,
        parent: &str,
        child_thread_id: &str,
        status: &str,
        cx: &mut Context<Self>,
    ) {
        for (id, surface) in self.retained_documents() {
            let Ok(view) = surface.view.clone().downcast::<Chat>() else {
                continue;
            };
            let Some(target) = view.read(cx).readonly_target() else {
                continue;
            };
            if (target.parent != parent && target.owner != parent)
                || (view.read(cx).readonly_child_thread_id() != Some(child_thread_id)
                    && !self.subagent_matches(target, parent, child_thread_id, cx))
            {
                continue;
            }
            let visible =
                self.settings.is_none() && self.layout.visible().any(|visible| visible == id);
            view.update(cx, |chat, cx| {
                chat.readonly_source_changed(
                    Some(crate::transcript::subagent::status(status).to_owned()),
                    false,
                    visible,
                    cx,
                )
            });
        }
    }

    fn subagent_thread(&self, target: &Target, cx: &App) -> Option<String> {
        let parent = self.subagent_parent(&target.parent, cx)?;
        parent
            .read(cx)
            .session
            .transcript
            .entries()
            .iter()
            .rev()
            .find_map(|entry| {
                entry
                    .subagent
                    .as_ref()
                    .filter(|agent| agent.session.as_ref() == Some(&target.session))
                    .map(|agent| agent.child_thread_id.clone())
                    .or_else(|| {
                        let tool = entry.tool.as_ref()?;
                        (crate::transcript::subagent::task_session(tool).as_ref()
                            == Some(&target.session))
                        .then(|| {
                            tool.metadata
                                .get("child_thread_id")?
                                .as_str()
                                .map(str::to_owned)
                        })
                        .flatten()
                    })
            })
    }

    fn subagent_parent(&self, parent: &str, cx: &App) -> Option<Entity<Chat>> {
        self.chats
            .get(parent)
            .map(|chat| chat.view.clone())
            .or_else(|| {
                self.retained_documents().find_map(|(_, surface)| {
                    let chat = surface.view.clone().downcast::<Chat>().ok()?;
                    chat.read(cx)
                        .readonly_target()
                        .is_some_and(|target| {
                            crate::tabs::chat(&target.session.session_dir) == parent
                        })
                        .then_some(chat)
                })
            })
    }

    fn subagent_matches(&self, target: &Target, parent: &str, child: &str, cx: &App) -> bool {
        self.chats.get(parent).is_some_and(|chat| {
            chat.view
                .read(cx)
                .session
                .transcript
                .entries()
                .iter()
                .any(|entry| {
                    entry.subagent.as_ref().is_some_and(|agent| {
                        agent.child_thread_id == child
                            && agent.session.as_ref() == Some(&target.session)
                    }) || entry.tool.as_ref().is_some_and(|tool| {
                        tool.metadata
                            .get("child_thread_id")
                            .and_then(|id| id.as_str())
                            == Some(child)
                            && crate::transcript::subagent::task_session(tool).as_ref()
                                == Some(&target.session)
                    })
                })
        })
    }

    pub(super) fn refresh_subagent_activity(
        &self,
        dir: &std::path::Path,
        status: proteus_contracts::app_protocol::AppSessionActivityStatus,
        cx: &mut Context<Self>,
    ) {
        for (id, surface) in self.retained_documents() {
            let Ok(view) = surface.view.clone().downcast::<Chat>() else {
                continue;
            };
            if !view
                .read(cx)
                .readonly_target()
                .is_some_and(|target| target.session.session_dir == dir)
            {
                continue;
            }
            let visible = self.settings.is_none() && self.layout.visible().any(|shown| shown == id);
            view.update(cx, |chat, cx| {
                chat.readonly_source_changed(
                    Some(activity_source(status).into()),
                    false,
                    visible,
                    cx,
                )
            });
        }
    }

    fn subagent_source(&self, target: &Target, cx: &App) -> Option<String> {
        let activity = || {
            self.sessions
                .iter()
                .find(|summary| {
                    summary.key.proteus_dir() == Some(target.session.session_dir.as_path())
                        && summary.proteus_session_id == Some(target.session.session_id)
                })
                .and_then(|summary| summary.activity.as_ref())
                .map(|activity| activity_source(activity.status).to_owned())
        };
        let Some(parent_chat) = self.subagent_parent(&target.parent, cx) else {
            return activity();
        };
        // Inspect only lifecycle/status metadata. Token deltas and copied
        // tool output never cause repeated journal-index rebuilds.
        let entries = parent_chat.read(cx).session.transcript.entries();
        let lifecycle = entries
            .iter()
            .rev()
            .find_map(|entry| {
                entry.subagent.as_ref().and_then(|agent| {
                    agent
                        .session
                        .as_ref()
                        .filter(|bound| *bound == &target.session)
                        .map(|_| crate::transcript::subagent::status(&agent.status).to_owned())
                })
            })
            .or_else(|| {
                entries.iter().rev().find_map(|entry| {
                    let tool = entry.tool.as_ref()?;
                    crate::transcript::subagent::task_session(tool)
                        .filter(|bound| bound == &target.session)
                        .map(|_| crate::transcript::subagent::task_status(tool).to_owned())
                })
            });
        if lifecycle
            .as_deref()
            .is_some_and(|status| matches!(status, "completed" | "failed" | "cancelled"))
        {
            lifecycle
        } else {
            activity().or(lifecycle)
        }
    }
}

fn activity_source(
    status: proteus_contracts::app_protocol::AppSessionActivityStatus,
) -> &'static str {
    use proteus_contracts::app_protocol::AppSessionActivityStatus::*;
    match status {
        Running => "running",
        WaitingApproval => "waiting_approval",
        WaitingInput => "waiting_input",
        Idle => "idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs;

    #[::core::prelude::v1::test]
    fn sidebar_child_uses_catalog_identity_without_guessing_a_task_thread() {
        let summary = |id: u128, parent: Option<u128>| {
            let mut summary = proteus_contracts::app_protocol::AppSessionSummary::new(
                format!("/saved/{id}").into(),
                uuid::Uuid::from_u128(id),
                "/project".into(),
                0,
                None,
                None,
            );
            summary.parent_session_id = parent.map(uuid::Uuid::from_u128);
            summary
        };
        let summaries =
            crate::conversations::from_proteus(vec![summary(1, None), summary(2, Some(1))]);
        let root = summaries[0].key.tab_id();
        let target = catalog_target(&summaries, &summaries[1].key, root.clone()).unwrap();
        assert_eq!(target.owner, root);
        assert_eq!(target.parent, root);
        assert_eq!(target.child_thread_id, None);
        assert_eq!(target.session.session_id, uuid::Uuid::from_u128(2));
        assert_eq!(target.session.session_dir, std::path::Path::new("/saved/2"));
        assert_eq!(Target::parse(&target.id()), Some(target));
        assert!(catalog_target(&summaries, &summaries[0].key, root).is_none());
    }
    #[::core::prelude::v1::test]
    fn child_document_reuses_right_pane_and_keeps_parent_command_context() {
        let parent = tabs::chat(std::path::Path::new("/saved/parent"));
        let target = Target {
            owner: parent.clone(),
            parent: parent.clone(),
            child_thread_id: Some("logical-task".into()),
            session: AppSubagentSession {
                session_id: uuid::Uuid::from_u128(2),
                session_dir: "/saved/child".into(),
            },
        };
        let mut layout = tabs::TabLayout::default();
        layout.open(&parent, |_| false);
        let pane = layout.focused;
        open_right_document(&mut layout, &target.id(), pane);
        let child_pane = layout.pane_of(&target.id()).unwrap();
        assert_eq!(layout.neighbor(pane, Edge::Right), Some(child_pane));
        layout.open(&target.id(), |_| true);
        assert_eq!(layout.pane_count(), 2);
        assert_eq!(layout.active_chat(Some(&parent)), Some(parent));
        assert!(!tabs::is_chat(&target.id()));
    }

    #[::core::prelude::v1::test]
    fn nested_source_parent_is_separate_from_workspace_owner() {
        let root = tabs::chat(std::path::Path::new("/saved/root"));
        let parent = tabs::chat(std::path::Path::new("/saved/normal-child"));
        let mut spaces = tabs::ChatSpaces::default();
        spaces.select(&root);
        let mut layout = spaces.active_layout().clone();
        layout.open(&parent, |_| true);
        spaces.save_active(layout);
        let target = Target {
            owner: spaces.owner_of_chat(&parent).unwrap().to_owned(),
            parent: parent.clone(),
            child_thread_id: Some("logical-grandchild".into()),
            session: AppSubagentSession {
                session_id: uuid::Uuid::from_u128(3),
                session_dir: "/saved/grandchild".into(),
            },
        };
        assert_eq!(target.owner, root);
        assert_eq!(target.parent, parent);
        assert_eq!(spaces.active_root(), Some(target.owner.as_str()));
        assert_eq!(Target::parse(&target.id()), Some(target));
    }
}
