use proteus_contracts::domain::{Event, ToolCall, ToolResult};
use uuid::Uuid;

use super::*;

fn message(role: &str, id: Option<MessageId>, text: &str) -> AppTranscriptMessage {
    AppTranscriptMessage {
        message_id: id,
        phase: None,
        role: role.to_owned(),
        text: text.to_owned(),
        images: Vec::new(),
        tool: None,
        subagent: None,
        streaming: false,
    }
}

fn delta(message_id: MessageId, offset: usize, text: &str) -> Event {
    Event::AssistantTextDelta {
        offset,
        message_id,
        phase: None,
        text: text.to_owned(),
    }
}

#[test]
fn deltas_follow_offsets_and_completion_settles_text() {
    let mut transcript = Transcript::default();
    let thread = Uuid::new_v4();
    let id = Uuid::new_v4();

    assert_eq!(
        transcript.apply_runtime(thread, &delta(id, 0, "Hel")),
        Change::Pushed
    );
    assert_eq!(
        transcript.apply_runtime(thread, &delta(id, 3, "lo")),
        Change::Updated(0)
    );
    // A replayed delta overwrites from its offset instead of duplicating.
    transcript.apply_runtime(thread, &delta(id, 3, "lo"));
    assert_eq!(&*transcript.entries()[0].text, "Hello");
    assert!(transcript.entries()[0].streaming);

    let completed = Event::AssistantMessageCompleted {
        message_id: id,
        phase: None,
        text: "Hello!".to_owned(),
    };
    transcript.apply_runtime(thread, &completed);
    assert_eq!(&*transcript.entries()[0].text, "Hello!");
    assert!(!transcript.entries()[0].streaming);
}

#[test]
fn foreign_thread_text_stays_out_of_the_turn() {
    let mut transcript = Transcript::default();
    let turn = Uuid::new_v4();
    let child = Uuid::new_v4();
    transcript.apply_runtime(
        turn,
        &Event::TurnStarted {
            session_id: Uuid::new_v4(),
            thread_id: turn,
            turn_id: Uuid::new_v4(),
        },
    );
    assert_eq!(
        transcript.apply_runtime(child, &delta(Uuid::new_v4(), 0, "x")),
        Change::None
    );
    assert!(transcript.entries().is_empty());
}

#[test]
fn subagent_tools_group_under_the_card() {
    let mut transcript = Transcript::default();
    let turn = Uuid::new_v4();
    let child = Uuid::new_v4();
    transcript.apply_runtime(
        turn,
        &Event::SubagentStarted {
            role: "explore".to_owned(),
            description: None,
            child_thread_id: child,
        },
    );
    let call = ToolCall::new("c1", "read", serde_json::json!({"path": "a"}));
    assert_eq!(
        transcript.apply_runtime(child, &Event::ToolCallRequested { call }),
        Change::Updated(0)
    );
    let result = ToolResult::ok("c1".to_owned(), "body");
    assert_eq!(
        transcript.apply_runtime(child, &Event::ToolFinished { result }),
        Change::Updated(0)
    );
    let subagent = transcript.entries()[0].subagent.as_ref().unwrap();
    assert_eq!(subagent.tools.len(), 1);
    assert_eq!(subagent.tools[0].status, "done");
    assert_eq!(transcript.entries().len(), 1);
}

#[test]
fn rootless_snapshot_keeps_turn_started_bootstrap_and_later_turn_switching() {
    let mut transcript = Transcript::default();
    transcript.set_root_thread(Some(Uuid::from_u128(99)));
    transcript.set_root_thread(None);
    for thread in [Uuid::from_u128(10), Uuid::from_u128(20)] {
        assert_eq!(
            transcript.apply_runtime(
                thread,
                &Event::TurnStarted {
                    session_id: Uuid::from_u128(1),
                    thread_id: thread,
                    turn_id: Uuid::new_v4(),
                }
            ),
            Change::None
        );
        assert_eq!(transcript.turn_thread, Some(thread));
        assert!(!transcript.snapshot_root_known);
        assert_eq!(
            transcript.apply_runtime(thread, &delta(Uuid::new_v4(), 0, "current turn")),
            Change::Pushed
        );
        assert_eq!(
            transcript.apply_runtime(Uuid::from_u128(30), &delta(Uuid::new_v4(), 0, "foreign")),
            Change::None
        );
    }
    assert_eq!(transcript.entries().len(), 2);
}

#[test]
fn snapshot_reuses_keys_and_text_of_known_messages() {
    let mut transcript = Transcript::default();
    let id = Uuid::new_v4();
    transcript.reset(vec![
        message("user", None, "hi"),
        message("assistant", Some(id), "answer"),
    ]);
    let key = transcript.entries()[1].key;
    let text = transcript.entries()[1].text.clone();

    let same = vec![
        message("user", None, "hi"),
        message("assistant", Some(id), "answer"),
    ];
    assert_eq!(transcript.reset(same), Change::None);

    let change = transcript.reset(vec![message("assistant", Some(id), "answer")]);
    assert_eq!(
        change,
        Change::Spliced {
            start: 0,
            removed: 2,
            inserted: 1
        }
    );
    assert_eq!(transcript.entries()[0].key, key);
    assert!(Arc::ptr_eq(&transcript.entries()[0].text, &text));
}
