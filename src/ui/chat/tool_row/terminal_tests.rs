//! The real tool-button handler runs while GPUI has leased its source Chat.
use super::*;
use crate::{api::Api, tabs::PaneId, ui::workspace::Workspace};
use std::{path::PathBuf, rc::Rc};

fn terminal() -> ToolTerminalRef {
    ToolTerminalRef {
        id: uuid::Uuid::from_u128(77),
        title: "synthetic command".into(),
        tty: true,
    }
}

fn context() -> ::gpui::TestAppContext {
    let cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    cx
}

fn source_chat(
    workspace: &Entity<Workspace>,
    api: Api,
    session: PathBuf,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Chat> {
    cx.new(|cx| {
        Chat::base(
            Some(api),
            workspace.downgrade(),
            super::super::Session::new(session),
            Rc::new(|_, _, _| None),
            window,
            cx,
        )
    })
}

#[::core::prelude::v1::test]
fn terminal_button_defers_until_source_chat_lease_ends_in_locked_and_shared_panes() {
    for existing_terminal in [false, true] {
        let mut cx = context();
        let window = cx.add_empty_window();
        let api = Api::new("http://127.0.0.1:1", "synthetic-never-used");
        let session = PathBuf::from("/synthetic/terminal-source");
        let owner = crate::tabs::chat(&session);
        let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
        let chat = window
            .update(|window, cx| source_chat(&workspace, api.clone(), session.clone(), window, cx));
        window.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.test_set_api(api);
                workspace.spaces.select(&owner);
                workspace.layout = workspace.spaces.active_layout().clone();
                workspace.layout.open(&owner, |_| false);
                workspace.spaces.save_active(workspace.layout.clone());
                workspace.active_chat = Some(owner.clone());
                workspace.bind_chat(&owner, chat.clone(), cx);
                if existing_terminal {
                    workspace.open_agent_terminal(owner.clone(), session, terminal(), window, cx);
                    let id = workspace
                        .layout
                        .panes()
                        .into_iter()
                        .flat_map(|pane| pane.tabs.iter())
                        .find(|id| id.starts_with("agent-terminal:"))
                        .cloned()
                        .unwrap();
                    workspace.layout.move_tab(&id, PaneId(0), None);
                    workspace.layout.activate(PaneId(0), &owner);
                } else {
                    let side = "chat:/synthetic/side";
                    workspace.layout.open_in(side, PaneId(0), |_| false);
                    workspace
                        .layout
                        .split(side, PaneId(0), crate::tabs::Edge::Right);
                    workspace.layout.activate(PaneId(0), &owner);
                    workspace.layout.lock_pane(PaneId(0), true, 1200., 800., 0.);
                    assert!(workspace.layout.is_locked(PaneId(0)));
                }
                workspace.spaces.save_active(workspace.layout.clone());
            });
        });
        window.update(|window, cx| {
            chat.update(cx, |chat, cx| {
                chat.open_tool_terminal(terminal(), window, cx);
                assert_eq!(
                    workspace.read(cx).layout.active(PaneId(0)),
                    Some(owner.as_str()),
                    "opening must wait until the source Chat is no longer leased"
                );
            });
        });
        cx.run_until_parked();
        workspace.read_with(&cx, |workspace, _| {
            assert!(workspace.layout.contains(&owner));
            assert!(
                workspace
                    .layout
                    .active(PaneId(0))
                    .is_some_and(|id| id.starts_with("agent-terminal:"))
            );
            assert!(workspace.chats.contains_key(&owner));
        });
    }
}

#[::core::prelude::v1::test]
fn deferred_terminal_click_cannot_replace_new_navigation_connection_or_source_chat() {
    for changed in ["navigation", "connection", "source"] {
        let mut cx = context();
        let window = cx.add_empty_window();
        let api = Api::new("http://127.0.0.1:1", "synthetic-never-used");
        let session = PathBuf::from("/synthetic/terminal-source");
        let owner = crate::tabs::chat(&session);
        let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
        let chat = window
            .update(|window, cx| source_chat(&workspace, api.clone(), session.clone(), window, cx));
        window.update(|_, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.test_set_api(api.clone());
                workspace.spaces.select(&owner);
                workspace.layout = workspace.spaces.active_layout().clone();
                workspace.layout.open(&owner, |_| false);
                workspace.spaces.save_active(workspace.layout.clone());
                workspace.active_chat = Some(owner.clone());
                workspace.bind_chat(&owner, chat.clone(), cx);
            });
        });
        window.update(|window, cx| {
            chat.update(cx, |chat, cx| {
                chat.open_tool_terminal(terminal(), window, cx)
            });
            let replacement = (changed == "source")
                .then(|| source_chat(&workspace, api.clone(), session.clone(), window, cx));
            workspace.update(cx, |workspace, cx| match changed {
                "navigation" => workspace.invalidate_navigation(),
                "connection" => {
                    workspace.test_set_api(Api::new("http://127.0.0.1:1", "replacement-connection"))
                }
                "source" => workspace.bind_chat(&owner, replacement.unwrap(), cx),
                _ => unreachable!(),
            });
        });
        cx.run_until_parked();
        workspace.read_with(&cx, |workspace, _| {
            assert_eq!(workspace.layout.active(PaneId(0)), Some(owner.as_str()));
            assert!(
                workspace.documents.is_empty(),
                "stale {changed} click created a viewer"
            );
            assert_eq!(workspace.layout.panes().len(), 1);
        });
    }
}
