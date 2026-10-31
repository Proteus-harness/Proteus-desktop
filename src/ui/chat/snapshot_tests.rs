//! Snapshot routing regressions through the same Session/History helper as the pump.
use super::{Session, history::History};
use crate::transcript::Change;
use proteus_contracts::app_protocol::{
    AppServerEvent, AppSessionSnapshot, AppTranscriptCursor, AppTranscriptMessage,
    AppTranscriptTool,
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

fn patch_row(
    args: serde_json::Value,
    effective_args: Option<serde_json::Value>,
) -> AppTranscriptMessage {
    AppTranscriptMessage {
        timestamp_ms: None,
        message_id: None,
        phase: None,
        role: "system".into(),
        text: String::new(),
        images: Vec::new(),
        tool: Some(AppTranscriptTool {
            call_id: "patch-call".into(),
            name: crate::tool_display::APPLY_PATCH.into(),
            args,
            effective_args,
            status: "done".into(),
            result: None,
            metadata: serde_json::Value::Null,
        }),
        subagent: None,
        streaming: false,
    }
}

fn displayed_patch<'a>(session: &'a Session, history: &'a History) -> Option<&'a str> {
    let entry = history.displayed(&session.transcript).entries().first()?;
    crate::tool_display::patch_text(super::tool_row::tool_args(entry.tool.as_ref()?))
}

#[test]
fn patch_source_follows_snapshots_while_the_row_key_stays_stable() {
    use serde_json::json;

    let (mut session, mut history) = session();
    let cases = [
        (json!({"patch": "raw-a"}), None, Some("raw-a")),
        (json!({"patch": "raw-b"}), None, Some("raw-b")),
        (
            json!({"patch": "raw-b"}),
            Some(json!({"input": "effective-a"})),
            Some("effective-a"),
        ),
        (
            json!({"patch": "raw-c"}),
            Some(json!({"input": "effective-b"})),
            Some("effective-b"),
        ),
        (
            json!({"patch": "raw-c"}),
            Some(json!({"patch": null, "input": "not-a-fallback"})),
            None,
        ),
        (
            json!({"patch": "raw-c"}),
            Some(json!({"patch": "  ", "input": "not-a-fallback"})),
            None,
        ),
        (json!({"input": "raw-d"}), None, Some("raw-d")),
    ];
    let mut key = None;
    for (index, (args, effective, expected)) in cases.into_iter().enumerate() {
        let mut revision = cursor(0);
        revision.revision = format!("revision-{index}");
        deliver_snapshot(
            &mut session,
            &mut history,
            snapshot(
                Some(id(10)),
                "first",
                index as u64 + 1,
                vec![patch_row(args, effective)],
                Some(revision),
            ),
        )
        .unwrap();
        let current = session.transcript.entries()[0].key;
        assert_eq!(current, *key.get_or_insert(current));
        // Reopening must select the current source, not a row-key/length cache.
        assert_eq!(displayed_patch(&session, &history), expected);
    }
}

#[test]
fn reconnect_patch_source_replaces_effective_args_without_replacing_the_row_key() {
    use serde_json::json;

    let (mut session, mut history) = session();
    let mut key = None;
    for (stream, seq, effective) in [("first", 9, "before"), ("restarted", 0, "after")] {
        session.cursor.begin_connection();
        deliver_snapshot(
            &mut session,
            &mut history,
            snapshot(
                Some(id(10)),
                stream,
                seq,
                vec![patch_row(
                    json!({"patch": "raw"}),
                    Some(json!({"patch": effective})),
                )],
                None,
            ),
        )
        .unwrap();
        let current = session.transcript.entries()[0].key;
        assert_eq!(current, *key.get_or_insert(current));
        assert_eq!(displayed_patch(&session, &history), Some(effective));
    }
}

fn execution_snapshot(
    root: Option<ThreadId>,
    stream: &str,
    seq: u64,
    active: Option<proteus_contracts::app_protocol::AppRun>,
    last: Option<proteus_contracts::app_protocol::AppRun>,
) -> AppServerEvent {
    let AppServerEvent::SessionSnapshot { mut snapshot } =
        snapshot(root, stream, seq, Vec::new(), None)
    else {
        unreachable!();
    };
    snapshot.execution = proteus_contracts::app_protocol::AppExecutionState { active, last };
    AppServerEvent::SessionSnapshot { snapshot }
}

fn app_run(
    run_id: &str,
    status: proteus_contracts::app_protocol::AppRunStatus,
) -> proteus_contracts::app_protocol::AppRun {
    proteus_contracts::app_protocol::AppRun {
        run_id: run_id.into(),
        options: Default::default(),
        status,
        error: None,
    }
}

/// Same accepted-baseline decision and ordering as the pump; no GPUI required.
fn deliver_settlement(session: &mut Session, history: &mut History, event: AppServerEvent) -> bool {
    if !session.cursor.accept(&event) {
        return false;
    }
    let AppServerEvent::SessionSnapshot { snapshot } = event else {
        unreachable!();
    };
    let settled = session.snapshot_settles_run(&snapshot);
    session.apply_snapshot(*snapshot, history);
    settled
}

#[test]
fn resync_confirms_known_terminal_run_once_without_requiring_stale_root() {
    use proteus_contracts::app_protocol::AppRunStatus;
    for status in [
        AppRunStatus::Success,
        AppRunStatus::Error,
        AppRunStatus::Canceled,
        AppRunStatus::Timeout,
    ] {
        let (mut session, mut history) = session();
        assert!(!deliver_settlement(
            &mut session,
            &mut history,
            execution_snapshot(
                Some(id(10)),
                "first",
                1,
                Some(app_run("known", AppRunStatus::Running)),
                None,
            ),
        ));
        assert!(
            session
                .cursor
                .accept(&AppServerEvent::EventStreamLagged { count: 5 })
        );
        assert!(deliver_settlement(
            &mut session,
            &mut history,
            execution_snapshot(None, "first", 7, None, Some(app_run("known", status))),
        ));
        assert!(!session.running());
        assert_eq!(session.root_thread, None);
        assert!(!deliver_settlement(
            &mut session,
            &mut history,
            execution_snapshot(
                None,
                "first",
                8,
                None,
                Some(app_run("known", AppRunStatus::Success))
            ),
        ));
    }
}

#[test]
fn reconnect_confirms_same_generation_but_never_a_restarted_generation() {
    use proteus_contracts::app_protocol::AppRunStatus;
    for (stream, expected) in [("first", true), ("restarted", false)] {
        let (mut session, mut history) = session();
        deliver_settlement(
            &mut session,
            &mut history,
            execution_snapshot(
                Some(id(10)),
                "first",
                1,
                Some(app_run("known", AppRunStatus::Running)),
                None,
            ),
        );
        session.cursor.begin_connection();
        assert_eq!(
            deliver_settlement(
                &mut session,
                &mut history,
                execution_snapshot(
                    Some(id(20)),
                    stream,
                    2,
                    None,
                    Some(app_run("known", AppRunStatus::Canceled))
                ),
            ),
            expected
        );
    }
}

#[test]
fn snapshot_does_not_invent_unknown_unrelated_or_still_active_settlement() {
    use proteus_contracts::app_protocol::AppRunStatus;
    for (known, last_id, last_status, next_active) in [
        (false, "known", AppRunStatus::Success, None),
        (true, "other", AppRunStatus::Success, None),
        (true, "known", AppRunStatus::Running, None),
        (true, "known", AppRunStatus::CancelRequested, None),
        (true, "known", AppRunStatus::Success, Some("next")),
        (true, "known", AppRunStatus::Success, Some("known")),
    ] {
        let (mut session, mut history) = session();
        deliver_settlement(
            &mut session,
            &mut history,
            execution_snapshot(
                None,
                "first",
                1,
                known.then(|| app_run("known", AppRunStatus::Running)),
                None,
            ),
        );
        assert!(!deliver_settlement(
            &mut session,
            &mut history,
            execution_snapshot(
                None,
                "first",
                2,
                next_active.map(|run| app_run(run, AppRunStatus::Running)),
                Some(app_run(last_id, last_status))
            ),
        ));
    }
}

#[test]
fn stale_foreign_or_already_delivered_execution_cannot_emit_settlement() {
    use proteus_contracts::app_protocol::AppRunStatus;
    let (mut session, mut history) = session();
    deliver_settlement(
        &mut session,
        &mut history,
        execution_snapshot(
            None,
            "first",
            3,
            Some(app_run("known", AppRunStatus::Running)),
            None,
        ),
    );
    for (stream, seq) in [("first", 2), ("other", 4), ("first", 3)] {
        assert!(!deliver_settlement(
            &mut session,
            &mut history,
            execution_snapshot(
                None,
                stream,
                seq,
                None,
                Some(app_run("known", AppRunStatus::Success))
            ),
        ));
        assert!(session.running());
    }
    // A terminal ExecutionUpdated already consumed the observed transition.
    session.execution.active = None;
    session.execution.last = Some(app_run("known", AppRunStatus::Success));
    assert!(!deliver_settlement(
        &mut session,
        &mut history,
        execution_snapshot(
            None,
            "first",
            4,
            None,
            Some(app_run("known", AppRunStatus::Success))
        ),
    ));
}

#[test]
fn same_run_id_from_another_session_is_not_a_settlement() {
    use proteus_contracts::app_protocol::AppRunStatus;
    let (mut session, mut history) = session();
    deliver_settlement(
        &mut session,
        &mut history,
        execution_snapshot(
            None,
            "first",
            1,
            Some(app_run("known", AppRunStatus::Running)),
            None,
        ),
    );
    session.cursor.begin_connection();
    let AppServerEvent::SessionSnapshot { mut snapshot } = execution_snapshot(
        None,
        "first",
        2,
        None,
        Some(app_run("known", AppRunStatus::Success)),
    ) else {
        unreachable!();
    };
    snapshot.session_id = id(99);
    assert!(!deliver_settlement(
        &mut session,
        &mut history,
        AppServerEvent::SessionSnapshot { snapshot },
    ));
}
