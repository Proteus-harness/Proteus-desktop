use proteus_contracts::app_protocol::AppServerEvent;
use proteus_contracts::domain::{EventContext, EventEnvelope};
use uuid::Uuid;

use super::*;

fn row(role: &str, id: MessageId, text: &str) -> AppTranscriptMessage {
    AppTranscriptMessage {
        message_id: Some(id),
        timestamp_ms: None,
        phase: None,
        role: role.into(),
        text: text.into(),
        images: Vec::new(),
        tool: None,
        subagent: None,
        streaming: false,
    }
}

fn reasoning(id: MessageId, offset: usize, text: &str) -> Event {
    Event::AssistantReasoningTextDelta {
        message_id: id,
        offset,
        text: text.into(),
    }
}

#[test]
fn reasoning_and_answer_keep_separate_identity_across_completion_and_snapshot() {
    let thread = Uuid::from_u128(10);
    let id = Uuid::from_u128(20);
    let mut transcript = Transcript::default();
    transcript.set_root_thread(Some(thread));
    assert_eq!(
        transcript.apply_runtime(thread, &reasoning(id, 0, "Пла")),
        Change::Pushed
    );
    transcript.apply_runtime(thread, &reasoning(id, 6, "н"));
    // Replayed byte offsets confirm the suffix without appending it twice.
    transcript.apply_runtime(thread, &reasoning(id, 6, "н"));
    let reasoning_key = transcript.entries()[0].key;
    assert_eq!(&*transcript.entries()[0].text, "План");
    transcript.apply_runtime(
        thread,
        &Event::AssistantTextDelta {
            message_id: id,
            phase: None,
            offset: 0,
            text: "Ответ".into(),
        },
    );
    let answer_key = transcript.entries()[1].key;
    transcript.apply_runtime(
        thread,
        &Event::AssistantReasoningMessageCompleted {
            message_id: id,
            text: "Полный план".into(),
        },
    );
    transcript.apply_runtime(
        thread,
        &Event::AssistantMessageCompleted {
            message_id: id,
            phase: None,
            text: "Ответ".into(),
        },
    );
    assert_eq!(transcript.entries().len(), 2);
    assert_eq!(&*transcript.entries()[0].text, "Полный план");
    assert_eq!(&*transcript.entries()[1].text, "Ответ");
    assert!(!transcript.entries().iter().any(|row| row.streaming));
    assert_eq!(
        transcript.reset(vec![
            row("reasoning", id, "Полный план"),
            row("assistant", id, "Ответ")
        ]),
        Change::None
    );
    assert_eq!(transcript.entries()[0].key, reasoning_key);
    assert_eq!(transcript.entries()[1].key, answer_key);
}

#[test]
fn reasoning_ignores_child_streams_and_keeps_repeated_requests_distinct() {
    let root = Uuid::from_u128(10);
    let id = Uuid::from_u128(20);
    let mut transcript = Transcript::default();
    transcript.set_root_thread(Some(root));
    assert_eq!(
        transcript.apply_runtime(Uuid::from_u128(11), &reasoning(id, 0, "child")),
        Change::None
    );
    assert_eq!(
        transcript.apply_runtime(root, &reasoning(id, 0, "")),
        Change::None
    );
    transcript.apply_runtime(root, &reasoning(id, 0, "first"));
    transcript.apply_runtime(root, &reasoning(Uuid::from_u128(21), 0, "second"));
    assert_eq!(transcript.entries().len(), 2);
    assert_eq!(&*transcript.entries()[0].text, "first");
    assert_eq!(&*transcript.entries()[1].text, "second");
    assert!(
        transcript
            .entries()
            .iter()
            .all(|row| row.role == Role::Reasoning)
    );
}

#[test]
fn empty_authoritative_reasoning_removes_its_disclosure_but_preserves_the_answer() {
    let thread = Uuid::from_u128(10);
    let id = Uuid::from_u128(20);
    let mut transcript = Transcript::default();
    transcript.apply_runtime(thread, &reasoning(id, 0, "provisional"));
    transcript.apply_runtime(
        thread,
        &Event::AssistantTextDelta {
            message_id: id,
            phase: None,
            offset: 0,
            text: "answer".into(),
        },
    );
    let answer_key = transcript.entries()[1].key;
    assert_eq!(
        transcript.apply_runtime(
            thread,
            &Event::AssistantReasoningMessageCompleted {
                message_id: id,
                text: String::new(),
            }
        ),
        Change::Spliced {
            start: 0,
            removed: 1,
            inserted: 0
        }
    );
    assert_eq!(transcript.entries().len(), 1);
    assert_eq!(transcript.entries()[0].key, answer_key);
    assert_eq!(transcript.entries()[0].role, Role::Assistant);
    assert_eq!(
        transcript.apply_runtime(
            thread,
            &Event::AssistantReasoningMessageCompleted {
                message_id: id,
                text: " \n".into(),
            }
        ),
        Change::None
    );
}

#[test]
fn reasoning_replay_preserves_later_suffix_and_rejects_conflicts_gaps_and_bad_utf8() {
    let thread = Uuid::from_u128(10);
    let id = Uuid::from_u128(20);
    let mut transcript = Transcript::default();
    assert_eq!(
        transcript.apply_runtime(thread, &reasoning(id, 4, "gap")),
        Change::None
    );
    transcript.apply_runtime(thread, &reasoning(id, 0, "План"));
    transcript.apply_runtime(thread, &reasoning(id, 8, " готов"));
    let key = transcript.entries()[0].key;
    assert_eq!(
        transcript.apply_runtime(thread, &reasoning(id, 0, "План")),
        Change::None
    );
    assert_eq!(&*transcript.entries()[0].text, "План готов");
    for (offset, text) in [(0, "Другой"), (1, "broken"), (50, "gap")] {
        assert_eq!(
            transcript.apply_runtime(thread, &reasoning(id, offset, text)),
            Change::None
        );
        assert_eq!(&*transcript.entries()[0].text, "План готов");
    }
    // Compatible overlap extends only the new tail, preserving prior bytes.
    assert_eq!(
        transcript.apply_runtime(thread, &reasoning(id, 8, " готов.")),
        Change::Updated(0)
    );
    assert_eq!(&*transcript.entries()[0].text, "План готов.");
    assert_eq!(transcript.entries()[0].key, key);
    // An overlap ending inside a code point is invalid even with offset zero.
    let other = Uuid::from_u128(21);
    transcript.apply_runtime(thread, &reasoning(other, 0, "ab"));
    assert_eq!(
        transcript.apply_runtime(thread, &reasoning(other, 0, "🙂")),
        Change::None
    );
    assert_eq!(&*transcript.entries()[1].text, "ab");
}

#[test]
fn late_reasoning_precedes_the_answer_and_keeps_inserted_time_and_snapshot_identity() {
    for completed_answer in [false, true] {
        let thread = Uuid::from_u128(10);
        let id = Uuid::from_u128(20);
        let mut transcript = Transcript::default();
        let answer = if completed_answer {
            Event::AssistantMessageCompleted {
                message_id: id,
                phase: None,
                text: "answer".into(),
            }
        } else {
            Event::AssistantTextDelta {
                message_id: id,
                phase: None,
                offset: 0,
                text: "answer".into(),
            }
        };
        let change = transcript.apply_runtime(thread, &answer);
        transcript.stamp(change, 10);
        let answer_key = transcript.entries()[0].key;
        let change = transcript.apply_runtime(
            thread,
            &Event::AssistantReasoningMessageCompleted {
                message_id: id,
                text: "late reasoning".into(),
            },
        );
        assert_eq!(
            change,
            Change::Spliced {
                start: 0,
                removed: 0,
                inserted: 1
            }
        );
        transcript.stamp(change, 20);
        let reasoning_key = transcript.entries()[0].key;
        assert_eq!(transcript.entries()[0].role, Role::Reasoning);
        assert_eq!(transcript.entries()[0].time, Some(20));
        assert_eq!(transcript.entries()[1].key, answer_key);
        assert_eq!(transcript.entries()[1].time, Some(10));
        assert_eq!(
            transcript.reset(vec![
                row("reasoning", id, "late reasoning"),
                row("assistant", id, "answer")
            ]),
            Change::None
        );
        assert_eq!(transcript.entries()[0].key, reasoning_key);
        assert_eq!(transcript.entries()[1].key, answer_key);
    }
}

#[test]
fn batched_answer_deltas_do_not_overwrite_reasoning_with_the_same_message_id() {
    let thread = Uuid::from_u128(10);
    let id = Uuid::from_u128(20);
    let mut transcript = Transcript::default();
    transcript.set_root_thread(Some(thread));
    transcript.apply_runtime(thread, &reasoning(id, 0, "thought"));
    let event = |offset, text: &str| {
        Box::new(AppServerEvent::Runtime {
            envelope: Box::new(EventEnvelope::new(
                EventContext::new(Uuid::from_u128(1), thread, Some(Uuid::from_u128(3))),
                offset as u64,
                Event::AssistantTextDelta {
                    message_id: id,
                    phase: None,
                    offset,
                    text: text.into(),
                },
            )),
        })
    };
    let mut run = TextRun::new(event(0, "an")).unwrap();
    run.push(event(2, "swer"));
    transcript.apply_text_run(&run);
    assert_eq!(transcript.entries().len(), 2);
    assert_eq!(&*transcript.entries()[0].text, "thought");
    assert_eq!(&*transcript.entries()[1].text, "answer");
    assert_eq!(transcript.entries()[1].role, Role::Assistant);
}

#[test]
fn native_codex_reasoning_uses_the_same_display_role() {
    use crate::agents::codex::{Item, ItemKind};
    let mut transcript = Transcript::default();
    transcript.external_upsert(Item {
        id: "reasoning-item".into(),
        turn_id: "turn".into(),
        status: "inProgress".into(),
        timestamp_ms: None,
        kind: ItemKind::Reasoning {
            text: "native".into(),
        },
    });
    let key = transcript.entries()[0].key;
    transcript.external_delta("turn", "reasoning-item", " thought", true, false);
    assert_eq!(transcript.entries()[0].role, Role::Reasoning);
    assert_eq!(transcript.entries()[0].key, key);
    assert_eq!(&*transcript.entries()[0].text, "native thought");
}
