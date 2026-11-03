//! Saved child chats use document tabs, so focusing one cannot address commands
//! to a foreign, unresumed session. Only the parent owns the workspace runtime.
use std::rc::Rc;

use gpui_kit::*;
use proteus_contracts::app_protocol::AppSubagentSession;

use super::super::chat::{Chat, readonly::Target};
use super::super::extensions::SurfaceView;
use super::super::workspace::Workspace;
use crate::tabs::{Edge, PaneId, TabLayout};

fn open_right_document(layout: &mut TabLayout, id: &str, parent: PaneId) {
    // split moves an existing tab. Add it to the parent's pane before moving
    // it right, all within one layout update so no intermediate frame paints.
    layout.open_in(id, parent, |_| false);
    layout.keep(id);
    layout.split(id, parent, Edge::Right);
}

impl Workspace {
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
        cx.defer(move |cx| {
            window
                .update(cx, |_, window, cx| {
                    workspace
                        .update(cx, |this, cx| {
                            this.open_subagent(owner, parent, child_thread_id, session, window, cx)
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
            child_thread_id,
            session,
        };
        let id = target.id();
        if self.show_external_tab(&id, cx) {
            return;
        }
        self.close_settings(cx);
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
        if let Some(surface) = self.documents.get(id) {
            if let Ok(chat) = surface.view.clone().downcast::<Chat>() {
                chat.update(cx, |chat, cx| {
                    chat.readonly_source_changed(source, false, true, cx)
                });
            }
            return Some(surface.clone());
        }
        let api = self.api()?;
        let owner = cx.weak_entity();
        let extras = self.composer_extras(cx);
        let chat =
            cx.new(|cx| Chat::saved_subagent(api, owner, target, source, extras, window, cx));
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
            let visible =
                self.settings.is_none() && self.layout.visible().any(|visible| visible == id);
            view.update(cx, |chat, cx| {
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
            if target.parent != parent || target.child_thread_id != child_thread_id {
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

    fn subagent_source(&self, target: &Target, cx: &App) -> Option<String> {
        let parent_chat = self.chats.get(&target.parent)?;
        // Inspect only lifecycle/status metadata. Token deltas and copied
        // tool output never cause repeated journal-index rebuilds.
        let entries = parent_chat.view.read(cx).session.transcript.entries();
        entries
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
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tabs;
    #[::core::prelude::v1::test]
    fn child_document_reuses_right_pane_and_keeps_parent_command_context() {
        let parent = tabs::chat(std::path::Path::new("/saved/parent"));
        let target = Target {
            owner: parent.clone(),
            parent: parent.clone(),
            child_thread_id: "logical-task".into(),
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
            child_thread_id: "logical-grandchild".into(),
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
