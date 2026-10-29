//! Snapshot routing regressions through the same Session/History helper as the pump.
use super::{Session, history::History};
use crate::transcript::Change;
use proteus_contracts::app_protocol::{
    AppServerEvent, AppSessionSnapshot, AppTranscriptCursor, AppTranscriptMessage,
};
use proteus_contracts::domain::{Event, EventContext, EventEnvelope, MessageId, ThreadId};
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn session() -> (Session, History) {
    let mut session = Session::new(Default::default());
    session.cursor.begin_connection();
    (session, History::default())
}

fn snapshot(
    root: Option<ThreadId>,
    stream: &str,
    seq: u64,
    transcript: Vec<AppTranscriptMessage>,
    history: Option<AppTranscriptCursor>,
) -> AppServerEvent {
    AppServerEvent::SessionSnapshot {
        snapshot: Box::new(AppSessionSnapshot {
            session_id: id(1),
            stream_id: stream.into(),
            seq,
            root_thread_id: root,
            transcript,
            history,
            execution: Default::default(),
        }),
    }
}

fn deliver_snapshot(
    session: &mut Session,
    history: &mut History,
    event: AppServerEvent,
) -> Option<(Change, Vec<Change>)> {
    if !session.cursor.accept(&event) {
        return None;
    }
    let AppServerEvent::SessionSnapshot { snapshot } = event else {
        panic!("expected snapshot");
    };
    Some(session.apply_snapshot(*snapshot, history))
}

fn runtime(session: &mut Session, thread: ThreadId, event: Event) -> Option<Change> {
    let event = AppServerEvent::Runtime {
        envelope: Box::new(EventEnvelope::new(
            EventContext::new(id(1), thread, None),
            0,
            event,
        )),
    };
    if !session.cursor.accept(&event) {
        return None;
    }
    let AppServerEvent::Runtime { envelope } = event else {
        unreachable!();
    };
    Some(
        session
            .transcript
            .apply_runtime(envelope.thread_id, &envelope.event),
    )
}

fn turn_started(session: &mut Session, root: ThreadId) {
    assert_eq!(
        runtime(
            session,
            root,
            Event::TurnStarted {
                session_id: id(1),
                thread_id: root,
                turn_id: id(3),
            },
        ),
        Some(Change::None),
    );
}

fn delta(message: MessageId, text: &str) -> Event {
    Event::AssistantTextDelta {
        offset: 0,
        message_id: message,
        phase: None,
        text: text.into(),
    }
}

fn completed(message: MessageId, text: &str) -> Event {
    Event::AssistantMessageCompleted {
        message_id: message,
        phase: None,
        text: text.into(),
    }
}

fn cursor(before: usize) -> AppTranscriptCursor {
    AppTranscriptCursor {
        revision: "same".into(),
        before,
        base: None,
    }
}

fn rows(start: usize, end: usize) -> Vec<AppTranscriptMessage> {
    (start..end)
        .map(|row| AppTranscriptMessage {
            timestamp_ms: None,
            message_id: Some(id(1000 + row as u128)),
            phase: None,
            role: "user".into(),
            text: format!("row {row}"),
            images: Vec::new(),
            tool: None,
            subagent: None,
            streaming: false,
        })
        .collect()
}

#[test]
fn snapshot_root_rejects_child_text_without_replayed_turn_started() {
    let (mut session, mut history) = session();
    let (root, child, message) = (id(10), id(11), id(100));
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(root), "first", 1, Vec::new(), None),
    )
    .unwrap();

    assert_eq!(
        runtime(&mut session, child, delta(message, "child delta")),
        Some(Change::None),
        "snapshot-routing regression: child delta entered parent baseline",
    );
    assert_eq!(
        runtime(&mut session, child, completed(message, "child completed")),
        Some(Change::None),
    );
    assert!(session.transcript.entries().is_empty());
    assert_eq!(
        runtime(&mut session, root, delta(message, "root")),
        Some(Change::Pushed),
    );
    assert_eq!(&*session.transcript.entries()[0].text, "root");
}

#[test]
fn replacement_snapshot_replaces_previous_root_routing() {
    let (mut session, mut history) = session();
    let (first, second) = (id(10), id(20));
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(first), "first", 1, Vec::new(), None),
    )
    .unwrap();
    turn_started(&mut session, first);
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(second), "first", 2, Vec::new(), None),
    )
    .unwrap();

    assert_eq!(session.root_thread, Some(second));
    assert_eq!(
        runtime(&mut session, first, delta(id(100), "old root")),
        Some(Change::None),
        "snapshot-routing regression: old root survived replacement baseline",
    );
    assert_eq!(
        runtime(&mut session, second, completed(id(101), "new root")),
        Some(Change::Pushed),
    );
    assert_eq!(session.transcript.entries().len(), 1);
    assert_eq!(&*session.transcript.entries()[0].text, "new root");
}

#[test]
fn reconnect_snapshot_establishes_root_before_any_new_turn_started() {
    let (mut session, mut history) = session();
    let (first, second) = (id(10), id(20));
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(first), "first", 9, Vec::new(), None),
    )
    .unwrap();
    turn_started(&mut session, first);

    session.cursor.begin_connection();
    assert_eq!(
        runtime(&mut session, second, delta(id(100), "before baseline")),
        None,
    );
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(second), "restarted", 0, Vec::new(), None),
    )
    .unwrap();
    assert_eq!(
        runtime(&mut session, first, delta(id(100), "old root")),
        Some(Change::None),
        "snapshot-routing regression: old root survived reconnect baseline",
    );
    assert_eq!(
        runtime(&mut session, second, delta(id(101), "new root")),
        Some(Change::Pushed),
    );
    assert_eq!(&*session.transcript.entries()[0].text, "new root");
}

#[test]
fn snapshot_without_root_clears_stale_root_routing() {
    let (mut session, mut history) = session();
    let first = id(10);
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(first), "first", 1, Vec::new(), None),
    )
    .unwrap();
    turn_started(&mut session, first);
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(None, "first", 2, Vec::new(), None),
    )
    .unwrap();
    assert_eq!(session.root_thread, None);
    // Preserve the existing unknown-root rule, rather than a stale old filter.
    assert_eq!(
        runtime(&mut session, id(20), delta(id(100), "no known root")),
        Some(Change::Pushed),
        "snapshot-routing regression: rootless baseline kept stale root",
    );
}

#[test]
fn unchanged_snapshot_replaces_root_and_keeps_bounded_live_row_identities() {
    let (mut session, mut history) = session();
    let (first, second) = (id(10), id(20));
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(first), "first", 1, rows(100, 150), Some(cursor(100))),
    )
    .unwrap();
    turn_started(&mut session, first);
    let keys: Vec<_> = session
        .transcript
        .entries()
        .iter()
        .map(|entry| entry.key)
        .collect();
    let (change, _) = deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(second), "first", 2, rows(100, 150), Some(cursor(100))),
    )
    .unwrap();
    assert_eq!(change, Change::None);
    assert_eq!(
        session
            .transcript
            .entries()
            .iter()
            .map(|entry| entry.key)
            .collect::<Vec<_>>(),
        keys
    );
    assert_eq!(
        runtime(&mut session, first, delta(id(100), "old root")),
        Some(Change::None)
    );
    assert_eq!(
        runtime(&mut session, second, delta(id(101), "new root")),
        Some(Change::Pushed)
    );
    assert_eq!(session.transcript.entries().len(), 51);
}

#[test]
fn rejected_snapshot_cannot_change_root_routing() {
    let (mut session, mut history) = session();
    let (first, second) = (id(10), id(20));
    deliver_snapshot(
        &mut session,
        &mut history,
        snapshot(Some(first), "first", 9, Vec::new(), None),
    )
    .unwrap();
    turn_started(&mut session, first);
    for rejected in [
        snapshot(Some(second), "other-stream", 10, Vec::new(), None),
        snapshot(Some(second), "first", 8, Vec::new(), None),
    ] {
        assert!(deliver_snapshot(&mut session, &mut history, rejected).is_none());
    }
    assert_eq!(session.root_thread, Some(first));
    assert_eq!(
        runtime(&mut session, second, delta(id(100), "wrong root")),
        Some(Change::None),
    );
    assert_eq!(
        runtime(&mut session, first, delta(id(101), "right root")),
        Some(Change::Pushed),
    );
}
