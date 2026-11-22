use ::gpui::TestAppContext;
use proteus_contracts::app_protocol::{
    AppExecutionState, AppRun, AppServerEvent, AppSessionSnapshot,
};
use proteus_contracts::domain::{Event, EventContext, EventEnvelope};
use std::rc::Rc;
use uuid::Uuid;

use super::*;
use crate::connection::PumpItem;
use crate::ui::chat::Session;

fn test_chat() -> (TestAppContext, Entity<Chat>) {
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
                Session::new("synthetic".into()),
                Rc::new(|_, _, _| None),
                window,
                cx,
            )
        })
    });
    chat.update(&mut cx, |chat, cx| {
        apply(
            chat,
            AppServerEvent::SessionSnapshot {
                snapshot: Box::new(AppSessionSnapshot {
                    session_id: Uuid::from_u128(3),
                    stream_id: "synthetic-stream".into(),
                    seq: 0,
                    root_thread_id: Some(Uuid::from_u128(1)),
                    history: None,
                    execution: Default::default(),
                    transcript: Vec::new(),
                }),
            },
            cx,
        );
    });
    (cx, chat)
}

fn apply(chat: &mut Chat, event: AppServerEvent, cx: &mut Context<Chat>) {
    chat.apply(PumpItem::Event(Box::new(event)), cx);
}

fn run(id: &str, status: AppRunStatus) -> AppRun {
    AppRun {
        run_id: id.into(),
        options: Default::default(),
        status,
        error: None,
    }
}

fn execution(active: Option<AppRun>, last: Option<AppRun>) -> AppServerEvent {
    AppServerEvent::ExecutionUpdated {
        execution: AppExecutionState { active, last },
    }
}

#[::core::prelude::v1::test]
fn persistent_notice_keeps_root_error_through_child_error_and_info_timeout() {
    let (mut app, chat) = test_chat();
    let root = Uuid::from_u128(1);
    let child = Uuid::from_u128(2);
    let failure = |thread, seq, message: &str| AppServerEvent::Runtime {
        envelope: Box::new(EventEnvelope::new(
            EventContext::new(Uuid::from_u128(3), thread, None),
            seq,
            Event::Error {
                message: message.into(),
            },
        )),
    };
    chat.update(&mut app, |chat, cx| {
        apply(chat, failure(root, 1, "root failure"), cx);
        apply(chat, failure(child, 2, "child failure"), cx);
        chat.show_notice("background update", cx);
        assert_eq!(
            chat.notice.error.as_ref().unwrap().text.as_ref(),
            "root failure"
        );
        assert!(chat.notice.info.is_some());
    });
    app.run_until_parked();
    app.executor().advance_clock(LONGEST);
    app.run_until_parked();
    chat.update(&mut app, |chat, cx| {
        assert!(chat.notice.info.is_none());
        assert_eq!(
            chat.notice.error.as_ref().unwrap().text.as_ref(),
            "root failure"
        );
        chat.dismiss_notice(Kind::Error, cx);
        assert!(chat.notice.error.is_none());
    });
}

#[::core::prelude::v1::test]
fn delayed_cancel_notice_tracks_the_same_authoritative_run_until_settlement() {
    let (mut app, chat) = test_chat();
    chat.update(&mut app, |chat, cx| {
        apply(
            chat,
            execution(Some(run("first", AppRunStatus::CancelRequested)), None),
            cx,
        );
        assert!(chat.notice.cancel.as_ref().unwrap().message.is_none());
    });
    app.run_until_parked();
    app.executor()
        .advance_clock(CANCEL_WAIT - Duration::from_secs(1));
    app.run_until_parked();
    chat.update(&mut app, |chat, cx| {
        apply(
            chat,
            execution(Some(run("first", AppRunStatus::CancelRequested)), None),
            cx,
        );
        assert!(chat.notice.cancel.as_ref().unwrap().message.is_none());
    });
    app.executor().advance_clock(Duration::from_secs(1));
    app.run_until_parked();
    chat.update(&mut app, |chat, cx| {
        assert_eq!(
            chat.notice
                .cancel
                .as_ref()
                .unwrap()
                .message
                .as_ref()
                .unwrap()
                .text
                .as_ref(),
            CANCEL_PENDING
        );
        assert!(chat.session.running());
        assert!(chat.session.execution.last.is_none());

        apply(
            chat,
            execution(None, Some(run("first", AppRunStatus::Canceled))),
            cx,
        );
        assert!(chat.notice.cancel.is_none());
        assert!(chat.notice.error.is_none());

        apply(
            chat,
            execution(Some(run("second", AppRunStatus::Running)), None),
            cx,
        );
        let mut failed = run("second", AppRunStatus::Error);
        failed.error = Some("provider failed".into());
        apply(chat, execution(None, Some(failed)), cx);
        assert_eq!(
            chat.notice.error.as_ref().unwrap().text.as_ref(),
            "provider failed"
        );
    });
}
