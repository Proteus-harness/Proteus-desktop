use super::*;
use crate::transcript::Change;
use crate::ui::chat::{Session, history::History};
use proteus_contracts::app_protocol::{
    AppExecutionState, AppPendingRequests, AppRun, AppRunStatus, AppServerEvent,
    AppSessionSnapshot, AppTranscriptMessage, AppTranscriptTool,
};
use proteus_contracts::domain::{Event, EventContext, EventEnvelope, ToolCall, ToolResult};
use proteus_contracts::model_standard::MessagePhase;
use serde_json::{Value, json};
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn event(event: Event, number: u64) -> Box<AppServerEvent> {
    let mut envelope =
        EventEnvelope::new(EventContext::new(id(1), id(10), Some(id(3))), number, event);
    envelope.event_id = id(10_000 + number as u128);
    envelope.timestamp_ms = number as i64;
    Box::new(AppServerEvent::Runtime {
        envelope: Box::new(envelope),
    })
}

fn text(number: u64) -> Box<AppServerEvent> {
    event(
        Event::AssistantTextDelta {
            offset: number as usize,
            message_id: id(100),
            phase: None,
            text: format!("delta {number}"),
        },
        number,
    )
}

fn snapshot(stream: &str, seq: u64, root: u128) -> AppServerEvent {
    AppServerEvent::SessionSnapshot {
        snapshot: Box::new(AppSessionSnapshot {
            session_id: id(1),
            stream_id: stream.into(),
            seq,
            root_thread_id: Some(id(root)),
            transcript: Vec::new(),
            history: None,
            execution: Default::default(),
        }),
    }
}

fn apply_snapshot(session: &mut Session, history: &mut History, event: AppServerEvent) {
    assert!(session.cursor.accept(&event));
    let AppServerEvent::SessionSnapshot { snapshot } = event else {
        panic!("expected snapshot");
    };
    session.apply_snapshot(*snapshot, history);
}

fn ready() -> (Session, History) {
    let mut session = Session::new(Default::default());
    session.cursor.begin_connection();
    let mut history = History::default();
    apply_snapshot(&mut session, &mut history, snapshot("first", 1, 10));
    (session, history)
}

#[test]
fn accepted_empty_text_still_hides_thinking_but_lagged_or_child_text_does_not() {
    let (mut session, mut history) = ready();
    session.connected = true;
    session.execution.active = Some(AppRun {
        run_id: "run".into(),
        options: Default::default(),
        status: AppRunStatus::Running,
        error: None,
    });
    session.feedback.execution(&session.execution);
    assert!(session.thinking());
    let empty = event(
        Event::AssistantTextDelta {
            offset: 0,
            message_id: id(100),
            phase: None,
            text: String::new(),
        },
        11,
    );
    let mut child = empty.clone();
    let AppServerEvent::Runtime { envelope } = child.as_mut() else {
        unreachable!()
    };
    envelope.thread_id = id(20);
    assert!(
        !session
            .apply_text_run(run(vec![child.clone(), child]))
            .shown()
    );
    assert!(session.thinking());
    session
        .cursor
        .accept(&AppServerEvent::EventStreamLagged { count: 1 });
    assert!(
        !session
            .apply_text_run(run(vec![empty.clone(), empty.clone()]))
            .shown()
    );
    assert!(session.thinking());
    let mut baseline = snapshot("first", 1, 10);
    let AppServerEvent::SessionSnapshot { snapshot } = &mut baseline else {
        unreachable!();
    };
    snapshot.execution = session.execution.clone();
    apply_snapshot(&mut session, &mut history, baseline);
    assert!(session.thinking());
    let outcome = session.apply_text_run(run(vec![empty.clone(), empty]));
    assert!(outcome.shown());
    assert_eq!(outcome.event_changes().count(), 0);
    assert!(session.transcript.entries().is_empty());
    assert!(!session.thinking());
}

fn run(events: Vec<Box<AppServerEvent>>) -> TextRun {
    let mut events = events.into_iter();
    let mut run = TextRun::new(events.next().unwrap()).unwrap();
    for event in events {
        run.push(event);
    }
    run
}

fn wire(item: &PumpItem) -> Value {
    match item {
        PumpItem::Connected => json!({"connection": "connected"}),
        PumpItem::Disconnected(error) => json!({"connection": "disconnected", "error": error}),
        PumpItem::Event(event) => serde_json::to_value(event).unwrap(),
    }
}

#[test]
fn representative_nondelta_events_flush_before_exact_barrier() {
    let call = ToolCall::new("call", "tool", json!({"a": 1}));
    let runtime = |event| PumpItem::Event(self::event(event, 40));
    let barriers = vec![
        PumpItem::Connected,
        PumpItem::Disconnected("offline".into()),
        PumpItem::Event(Box::new(snapshot("first", 1, 10))),
        PumpItem::Event(Box::new(AppServerEvent::ExecutionUpdated {
            execution: AppExecutionState {
                active: Some(AppRun {
                    run_id: "run".into(),
                    options: Default::default(),
                    status: AppRunStatus::CancelRequested,
                    error: None,
                }),
                last: None,
            },
        })),
        runtime(Event::TurnStarted {
            session_id: id(1),
            thread_id: id(10),
            turn_id: id(3),
        }),
        runtime(Event::AssistantMessageCompleted {
            message_id: id(100),
            phase: None,
            text: "completed".into(),
        }),
        runtime(Event::AssistantReasoningDelta {
            text: "ignored".into(),
        }),
        runtime(Event::AssistantToolArgsDelta {
            call_id: "call".into(),
            args_delta: "{}".into(),
        }),
        runtime(Event::ToolCallRequested { call: call.clone() }),
        runtime(Event::ToolCallResolved { call }),
        runtime(Event::ApprovalRequested {
            call_id: "call".into(),
            reason: "review".into(),
        }),
        runtime(Event::ApprovalResolved {
            call_id: "call".into(),
            approved: true,
        }),
        runtime(Event::ToolFinished {
            result: ToolResult::ok("call".into(), "done"),
        }),
        runtime(Event::SubagentStarted {
            role: "child".into(),
            description: None,
            child_thread_id: id(20),
        }),
        PumpItem::Event(Box::new(AppServerEvent::PendingRequestsUpdated {
            snapshot: Box::new(AppPendingRequests::new(id(1), "first".into())),
        })),
        PumpItem::Event(Box::new(AppServerEvent::UserMessageSubmitted {
            text: "user".into(),
            images: Vec::new(),
        })),
        PumpItem::Event(Box::new(AppServerEvent::ApprovalResolved {
            approval_id: "approval".into(),
            approved: true,
        })),
        PumpItem::Event(Box::new(AppServerEvent::UserInputResolved {
            request_id: "input".into(),
        })),
        PumpItem::Event(Box::new(AppServerEvent::EventStreamLagged { count: 3 })),
        PumpItem::Event(Box::new(AppServerEvent::ProfileReloadStatus {
            error: None,
        })),
        PumpItem::Event(Box::new(AppServerEvent::ModulesReloaded {
            old_epoch: 1,
            new_epoch: 2,
            tool_names: Vec::new(),
        })),
        PumpItem::Event(Box::new(AppServerEvent::SessionCatalogUpdated)),
        PumpItem::Event(Box::new(AppServerEvent::Error {
            message: "error".into(),
        })),
        PumpItem::Event(Box::new(AppServerEvent::Shutdown)),
    ];
    for barrier in barriers {
        let items = vec![
            PumpItem::Event(text(11)),
            PumpItem::Event(text(12)),
            barrier,
            PumpItem::Event(text(13)),
            PumpItem::Event(text(14)),
        ];
        let expected: Vec<_> = items.iter().map(wire).collect();
        let mut flattened = Vec::new();
        let mut lengths = Vec::new();
        for item in batches(items) {
            match item {
                BatchItem::Text(run) => {
                    let events = run.into_events();
                    lengths.push(events.len());
                    flattened.extend(events.into_iter().map(PumpItem::Event));
                }
                BatchItem::Barrier(item) => {
                    lengths.push(0);
                    flattened.push(item);
                }
            }
        }
        assert_eq!(lengths, [2, 0, 2]);
        assert_eq!(flattened.iter().map(wire).collect::<Vec<_>>(), expected);
    }
}

#[test]
fn only_equal_context_and_phase_join_but_seq_and_timestamp_are_not_rewritten() {
    for field in 0..6 {
        let first = text(11);
        let mut second = text(12);
        let AppServerEvent::Runtime { envelope } = second.as_mut() else {
            unreachable!();
        };
        match field {
            0 => envelope.schema_version += 1,
            1 => envelope.session_id = id(2),
            2 => envelope.thread_id = id(20),
            3 => envelope.turn_id = Some(id(4)),
            4 | 5 => {
                let Event::AssistantTextDelta {
                    message_id, phase, ..
                } = &mut envelope.event
                else {
                    unreachable!();
                };
                if field == 4 {
                    *message_id = id(101);
                } else {
                    *phase = Some(MessagePhase::Commentary);
                }
            }
            _ => unreachable!(),
        }
        assert!(!TextRun::new(first).unwrap().accepts(&second));
        assert_eq!(
            batches(vec![PumpItem::Event(text(11)), PumpItem::Event(second)]).count(),
            2,
        );
    }
    let mut events = vec![text(11), text(12), text(13)];
    for (event, seq) in events.iter_mut().zip([9, 9, 1]) {
        let AppServerEvent::Runtime { envelope } = event.as_mut() else {
            unreachable!();
        };
        envelope.seq = seq;
    }
    let expected: Vec<_> = events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect();
    let run = run(events);
    assert_eq!(
        run.into_events()
            .iter()
            .map(|event| serde_json::to_value(event).unwrap())
            .collect::<Vec<_>>(),
        expected,
    );
}

#[test]
fn cursor_gates_lag_and_reconnect_before_root_routing() {
    let mut session = Session::new(Default::default());
    session.cursor.begin_connection();
    assert!(!session.apply_text_run(run(vec![text(11)])).shown());
    let mut history = History::default();
    apply_snapshot(&mut session, &mut history, snapshot("first", 1, 10));
    assert!(
        session
            .apply_text_run(run(vec![text(11), text(12)]))
            .shown()
    );
    assert!(
        session
            .cursor
            .accept(&AppServerEvent::EventStreamLagged { count: 1 })
    );
    assert!(!session.apply_text_run(run(vec![text(13)])).shown());
    apply_snapshot(&mut session, &mut history, snapshot("first", 1, 10));
    assert!(session.apply_text_run(run(vec![text(14)])).shown());

    session.cursor.begin_connection();
    assert!(!session.apply_text_run(run(vec![text(15)])).shown());
    apply_snapshot(&mut session, &mut history, snapshot("second", 0, 20));
    assert!(!session.apply_text_run(run(vec![text(16)])).shown());
    let mut new_root = text(17);
    let AppServerEvent::Runtime { envelope } = new_root.as_mut() else {
        unreachable!();
    };
    envelope.thread_id = id(20);
    assert!(session.apply_text_run(run(vec![new_root])).shown());

    let mut foreign = text(18);
    let AppServerEvent::Runtime { envelope } = foreign.as_mut() else {
        unreachable!();
    };
    envelope.session_id = id(2);
    envelope.thread_id = id(20);
    assert!(!session.apply_text_run(run(vec![foreign])).shown());
    assert_eq!(session.transcript.entries().len(), 1);
    assert_eq!(&*session.transcript.entries()[0].text, "delta 17");
}

fn projection(session: &Session, change: Change) -> Option<Value> {
    crate::ui::extensions::session::changed(session, change)
        .map(|update| serde_json::to_value(update).unwrap())
}

fn oracle(session: &mut Session, events: &[Box<AppServerEvent>]) -> (Vec<Change>, Vec<Value>) {
    let mut changes = Vec::new();
    let mut updates = Vec::new();
    for event in events {
        if !session.cursor.accept(event) {
            continue;
        }
        let AppServerEvent::Runtime { envelope } = event.as_ref() else {
            unreachable!();
        };
        let change = session
            .transcript
            .apply_runtime(envelope.thread_id, &envelope.event);
        session.transcript.stamp(change, envelope.timestamp_ms);
        if change != Change::None {
            changes.push(change);
        }
        if let Some(update) = projection(session, change) {
            updates.push(update);
        }
    }
    (changes, updates)
}

#[test]
fn original_observer_changes_and_public_metadata_projection_are_preserved() {
    for collision in [false, true] {
        let make = || {
            let (mut session, history) = ready();
            if collision {
                session.transcript.reset(vec![AppTranscriptMessage {
                    message_id: Some(id(100)),
                    phase: None,
                    role: "user".into(),
                    text: "original".into(),
                    images: Vec::new(),
                    tool: Some(AppTranscriptTool {
                        call_id: "collision".into(),
                        name: "tool".into(),
                        args: json!({"keep": 1}),
                        effective_args: None,
                        status: "done".into(),
                        result: Some("unchanged".into()),
                        metadata: json!({"source": "fixture"}),
                    }),
                    subagent: None,
                    streaming: false,
                }]);
            }
            (session, history)
        };
        let (mut actual, _) = make();
        let (mut expected, _) = make();
        let events = vec![text(11), text(12), text(13)];
        let (changes, updates) = oracle(&mut expected, &events);
        let outcome = actual.apply_text_run(run(events));
        assert_eq!(outcome.event_changes().collect::<Vec<_>>(), changes);
        assert_eq!(
            outcome
                .event_changes()
                .filter_map(|change| projection(&actual, change))
                .collect::<Vec<_>>(),
            updates,
        );
        assert_eq!(
            actual.transcript.entries().len(),
            expected.transcript.entries().len()
        );
        assert_eq!(
            actual.transcript.entries()[0].text,
            expected.transcript.entries()[0].text
        );
        assert_eq!(
            actual.transcript.entries()[0].time,
            expected.transcript.entries()[0].time
        );
    }
}

fn observed_rows(session: &Session) -> Vec<Value> {
    session
        .transcript
        .entries()
        .iter()
        .map(|entry| {
            json!({
                "key": entry.key,
                "id": entry.message_id,
                "role": format!("{:?}", entry.role),
                "text": &*entry.text,
                "images": entry.images,
                "tool": entry.tool,
                "subagent": entry.subagent,
                "streaming": entry.streaming,
                "time": entry.time,
            })
        })
        .collect()
}

#[test]
fn real_reducer_matches_at_completion_tool_and_turn_barriers() {
    let delta = |offset, value: &str, number, thread| {
        let mut event = event(
            Event::AssistantTextDelta {
                offset,
                message_id: id(100),
                phase: None,
                text: value.into(),
            },
            number,
        );
        let AppServerEvent::Runtime { envelope } = event.as_mut() else {
            unreachable!();
        };
        envelope.thread_id = id(thread);
        PumpItem::Event(event)
    };
    let mut turn = event(
        Event::TurnStarted {
            session_id: id(1),
            thread_id: id(20),
            turn_id: id(4),
        },
        17,
    );
    let AppServerEvent::Runtime { envelope } = turn.as_mut() else {
        unreachable!();
    };
    envelope.thread_id = id(20);
    let source = vec![
        delta(0, "a", 11, 10),
        delta(1, "b", 12, 10),
        PumpItem::Event(event(
            Event::AssistantMessageCompleted {
                message_id: id(100),
                phase: None,
                text: "done".into(),
            },
            13,
        )),
        delta(4, "", 14, 10),
        PumpItem::Event(event(
            Event::ToolCallRequested {
                call: ToolCall::new("barrier", "tool", json!({})),
            },
            15,
        )),
        delta(4, "!", 16, 10),
        PumpItem::Event(turn),
        delta(5, " root tail", 18, 10),
        delta(5, "ignored child", 19, 20),
        delta(13, " ignored child tail", 20, 20),
    ];
    let (mut actual, _) = ready();
    let (mut expected, _) = ready();
    let mut barriers = 0;
    for item in batches(source) {
        match item {
            BatchItem::Text(run) => {
                let events = run.clone().into_events();
                let (changes, updates) = oracle(&mut expected, &events);
                let outcome = actual.apply_text_run(run);
                assert_eq!(outcome.event_changes().collect::<Vec<_>>(), changes);
                assert_eq!(
                    outcome
                        .event_changes()
                        .filter_map(|change| projection(&actual, change))
                        .collect::<Vec<_>>(),
                    updates,
                );
            }
            BatchItem::Barrier(PumpItem::Event(event)) => {
                // Both paths execute the unchanged production reducer at a barrier.
                let (expected_changes, _) = oracle(&mut expected, &[event.clone()]);
                let (actual_changes, _) = oracle(&mut actual, &[event]);
                assert_eq!(actual_changes, expected_changes);
                barriers += 1;
            }
            BatchItem::Barrier(_) => unreachable!("runtime-only fixture"),
        }
        assert_eq!(observed_rows(&actual), observed_rows(&expected));
    }
    assert_eq!(barriers, 3);
    assert_eq!(&*actual.transcript.entries()[0].text, "done! root tail");
}

#[test]
fn child_turn_started_cannot_hijack_a_snapshot_root_or_colliding_message() {
    // The snapshot already contains the root's TurnStarted, so it is not replayed.
    let (mut session, _) = ready();
    let delta = |thread, offset, text: &str, number| {
        let mut event = event(
            Event::AssistantTextDelta {
                offset,
                message_id: id(100),
                phase: None,
                text: text.into(),
            },
            number,
        );
        let AppServerEvent::Runtime { envelope } = event.as_mut() else {
            unreachable!();
        };
        envelope.thread_id = id(thread);
        PumpItem::Event(event)
    };
    let mut child_start = event(
        Event::TurnStarted {
            session_id: id(1),
            thread_id: id(20),
            turn_id: id(4),
        },
        12,
    );
    let AppServerEvent::Runtime { envelope } = child_start.as_mut() else {
        unreachable!();
    };
    envelope.thread_id = id(20);
    let source = vec![
        delta(10, 0, "root", 11),
        PumpItem::Event(child_start),
        delta(20, 0, "CHILD", 13),
        delta(20, 5, " HIJACK", 14),
        delta(10, 4, " continued", 15),
        delta(10, 14, "!", 16),
    ];
    let mut text_batches = 0;
    for item in batches(source) {
        match item {
            BatchItem::Text(run) => {
                let outcome = session.apply_text_run(run);
                text_batches += 1;
                match text_batches {
                    1 => assert_eq!(&*session.transcript.entries()[0].text, "root"),
                    2 => {
                        assert!(!outcome.shown());
                        assert_eq!(&*session.transcript.entries()[0].text, "root");
                    }
                    3 => assert_eq!(&*session.transcript.entries()[0].text, "root continued!"),
                    _ => unreachable!(),
                }
            }
            BatchItem::Barrier(PumpItem::Event(event)) => {
                assert!(session.cursor.accept(&event));
                let AppServerEvent::Runtime { envelope } = event.as_ref() else {
                    unreachable!();
                };
                assert_eq!(
                    session
                        .transcript
                        .apply_runtime(envelope.thread_id, &envelope.event),
                    Change::None
                );
            }
            BatchItem::Barrier(_) => unreachable!(),
        }
    }
    assert_eq!(text_batches, 3);
    assert_eq!(session.transcript.entries().len(), 1);
    assert_eq!(session.transcript.entries()[0].time, Some(11));
}

mod layout;
