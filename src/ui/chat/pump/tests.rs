//! Session shutdown is not an app-wide backend failure.
use std::cell::RefCell;
use std::rc::Rc;

use ::gpui::TestAppContext;
use gpui_kit::*;
use proteus_contracts::app_protocol::{AppPendingRequests, AppRun, AppRunStatus};

use super::*;
use crate::api::Api;
use crate::ui::chat::{ChatState, ComposerExtras, Session};

#[::core::prelude::v1::test]
fn session_shutdown_only_emits_session_state_and_preserves_shared_api_identity() {
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = cx.add_empty_window();
    let api = Api::new("http://127.0.0.1:1", "synthetic-unused-token");
    let connection = api.connection_id();
    let chat = window.update(|window, cx| {
        cx.new(|cx| {
            let extras: ComposerExtras = Rc::new(|_, _, _| None);
            let mut session = Session::new("synthetic-session".into());
            session.stream_connected();
            session.execution.active = Some(AppRun {
                run_id: "observed-run".into(),
                options: Default::default(),
                status: AppRunStatus::Running,
                error: None,
            });
            session.feedback.execution(&session.execution);
            session.pending = Some(AppPendingRequests::new(
                uuid::Uuid::from_u128(1),
                "observed-stream".into(),
            ));
            Chat::base(
                // Reconnect exercises local state without issuing config HTTP.
                // The shared API identity is attached before session Shutdown.
                None,
                WeakEntity::new_invalid(),
                session,
                extras,
                window,
                cx,
            )
        })
    });
    let events = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let events = events.clone();
        cx.subscribe(&chat, move |_, event: &ChatEvent, _| {
            events
                .borrow_mut()
                .push(matches!(event, ChatEvent::SessionStateChanged(_)));
        })
    });
    cx.run_until_parked();

    // A failed connection is uncertain, not a completed/cancelled run.
    chat.update(&mut cx, |chat, cx| {
        chat.apply(PumpItem::Disconnected("os error 111".into()), cx);
        assert_eq!(chat.session.state(), ChatState::Trouble("Переподключение…"));
        assert_eq!(
            chat.session.execution.active.as_ref().unwrap().run_id,
            "observed-run"
        );
        assert!(chat.session.pending.is_some());
        assert!(!chat.session.thinking());
        chat.apply(PumpItem::Connected, cx);
        assert_eq!(chat.session.state(), ChatState::Busy("Работает…"));
        assert!(chat.session.link_error.is_none());
    });
    cx.run_until_parked();
    assert_eq!(&*events.borrow(), &[true, true]);
    events.borrow_mut().clear();

    chat.update(&mut cx, |chat, cx| {
        chat.api = Some(api);
        assert!(chat.apply(PumpItem::Event(Box::new(AppServerEvent::Shutdown)), cx));
        assert_eq!(chat.api.as_ref().unwrap().connection_id(), connection);
        assert_eq!(chat.session.state(), ChatState::Trouble("Сессия закрыта"));
        assert!(!chat.session.connected);
        assert!(!chat.session.thinking());
        assert!(chat.session.pending.is_none());
        // Shutdown does not fabricate a canonical outcome for the last run.
        assert_eq!(
            chat.session.execution.active.as_ref().unwrap().run_id,
            "observed-run"
        );
        assert!(chat.session.execution.last.is_none());
    });
    cx.run_until_parked();
    assert_eq!(
        &*events.borrow(),
        &[true],
        "session Shutdown must not emit an app-wide backend-stopped signal"
    );
}

#[::core::prelude::v1::test]
fn background_child_completion_notifies_saved_document_without_a_live_progress_row() {
    use proteus_contracts::app_protocol::AppSessionSnapshot;
    use proteus_contracts::domain::{EventContext, EventEnvelope};
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = cx.add_empty_window();
    let chat = window.update(|window, cx| {
        cx.new(|cx| {
            Chat::base(
                None,
                WeakEntity::new_invalid(),
                Session::new("parent".into()),
                Rc::new(|_, _, _| None),
                window,
                cx,
            )
        })
    });
    let completed = Rc::new(RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let completed = completed.clone();
        cx.subscribe(&chat, move |_, event: &ChatEvent, _| {
            if matches!(event, ChatEvent::CatalogUpdated) {
                completed
                    .borrow_mut()
                    .push(("catalog".into(), String::new()));
            }
            if let ChatEvent::SubagentLifecycle {
                child_thread_id,
                status,
            } = event
            {
                completed
                    .borrow_mut()
                    .push((child_thread_id.clone(), status.clone()));
            }
        })
    });
    let session = uuid::Uuid::from_u128(1);
    let root = uuid::Uuid::from_u128(2);
    let child = uuid::Uuid::from_u128(3);
    chat.update(&mut cx, |chat, cx| {
        chat.apply(PumpItem::Connected, cx);
        chat.apply(
            PumpItem::Event(Box::new(AppServerEvent::SessionSnapshot {
                snapshot: Box::new(AppSessionSnapshot {
                    session_id: session,
                    stream_id: "parent-stream".into(),
                    seq: 0,
                    root_thread_id: Some(root),
                    transcript: Vec::new(),
                    history: None,
                    execution: Default::default(),
                }),
            })),
            cx,
        );
        chat.apply(
            PumpItem::Event(Box::new(AppServerEvent::Runtime {
                envelope: Box::new(EventEnvelope::new(
                    EventContext::new(session, root, None),
                    1,
                    RuntimeEvent::SubagentSessionBound {
                        child_thread_id: child,
                        session_id: uuid::Uuid::from_u128(4),
                        session_dir: "/saved/exact-child".into(),
                    },
                )),
            })),
            cx,
        );
        chat.apply(
            PumpItem::Event(Box::new(AppServerEvent::Runtime {
                envelope: Box::new(EventEnvelope::new(
                    EventContext::new(session, root, None),
                    2,
                    RuntimeEvent::SubagentFinished {
                        role: "explorer".into(),
                        status: "completed".into(),
                        iterations: 2,
                        child_thread_id: child,
                    },
                )),
            })),
            cx,
        );
        assert!(
            chat.session.transcript.entries().is_empty(),
            "completion need not manufacture a progress row"
        );
    });
    cx.run_until_parked();
    assert_eq!(
        &*completed.borrow(),
        &[
            ("catalog".into(), String::new()),
            (child.to_string(), "completed".into())
        ]
    );
}
