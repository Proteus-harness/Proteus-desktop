use super::*;
use crate::transcript::{Entry, Role};
use proteus_contracts::app_protocol::{AppTranscriptMessage, AppTranscriptTool};
use proteus_contracts::domain::EventContext;
use uuid::Uuid;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn message(message_id: u128, text: &str) -> AppTranscriptMessage {
    AppTranscriptMessage {
        timestamp_ms: None,
        message_id: Some(id(message_id)),
        phase: None,
        role: "assistant".into(),
        text: text.into(),
        images: Vec::new(),
        tool: None,
        subagent: None,
        streaming: true,
    }
}

fn fixture(base: Option<&str>) -> Transcript {
    let mut transcript = Transcript::default();
    let mut rows = vec![message(200, "other row")];
    if let Some(base) = base {
        rows.push(message(100, base));
    }
    transcript.reset(rows);
    transcript.entries[0].time = Some(7);
    if base.is_some() {
        transcript.entries[1].streaming = false;
    }
    transcript.set_root_thread(Some(id(10)));
    transcript
}

fn event(offset: usize, text: &str, time: i64) -> Box<AppServerEvent> {
    let mut envelope = EventEnvelope::new(
        EventContext::new(id(1), id(10), Some(id(3))),
        time as u64,
        Event::AssistantTextDelta {
            offset,
            message_id: id(100),
            phase: None,
            text: text.into(),
        },
    );
    envelope.event_id = id(10_000 + time.unsigned_abs() as u128);
    envelope.timestamp_ms = time;
    Box::new(AppServerEvent::Runtime {
        envelope: Box::new(envelope),
    })
}

fn run(events: &[Box<AppServerEvent>]) -> TextRun {
    let mut run = TextRun::new(events[0].clone()).unwrap();
    for event in &events[1..] {
        run.push(event.clone());
    }
    run
}

/// The production single-event reducer is the oracle, never a copied splice.
fn apply_oracle(transcript: &mut Transcript, events: &[Box<AppServerEvent>]) -> Vec<Change> {
    events
        .iter()
        .map(|event| {
            let AppServerEvent::Runtime { envelope } = event.as_ref() else {
                panic!("expected runtime event");
            };
            let change = transcript.apply_runtime(envelope.thread_id, &envelope.event);
            transcript.stamp(change, envelope.timestamp_ms);
            change
        })
        .collect()
}

fn assert_entry(actual: &Entry, expected: &Entry) {
    assert_eq!(actual.key, expected.key);
    assert_eq!(actual.message_id, expected.message_id);
    assert_eq!(actual.role, expected.role);
    assert_eq!(actual.text, expected.text);
    assert_eq!(actual.images, expected.images);
    assert_eq!(
        actual
            .tool
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap()),
        expected
            .tool
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap()),
    );
    assert_eq!(
        actual
            .subagent
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap()),
        expected
            .subagent
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap()),
    );
    assert_eq!(actual.streaming, expected.streaming);
    assert_eq!(actual.time, expected.time);
}

fn assert_transcript(actual: &Transcript, expected: &Transcript) {
    assert_eq!(
        actual.next_key.load(std::sync::atomic::Ordering::Relaxed),
        expected.next_key.load(std::sync::atomic::Ordering::Relaxed),
    );
    assert_eq!(actual.turn_thread, expected.turn_thread);
    assert_eq!(actual.snapshot_root_known, expected.snapshot_root_known);
    assert_eq!(actual.entries.len(), expected.entries.len());
    for (actual, expected) in actual.entries.iter().zip(&expected.entries) {
        assert_entry(actual, expected);
    }
}

/// Compare after EVERY partition boundary, not just the final string.
fn compare_partition(base: Option<&str>, events: &[Box<AppServerEvent>], mask: usize) {
    let mut actual = fixture(base);
    let mut expected = fixture(base);
    let mut start = 0;
    for end in 1..=events.len() {
        if end != events.len() && mask & (1 << (end - 1)) == 0 {
            continue;
        }
        let expected_changes = apply_oracle(&mut expected, &events[start..end]);
        let outcome = actual.apply_text_run(&run(&events[start..end]));
        assert_eq!(outcome.changes.as_slice(), expected_changes);
        assert_eq!(
            outcome.event_changes().collect::<Vec<_>>(),
            expected_changes
                .iter()
                .copied()
                .filter(|change| *change != Change::None)
                .collect::<Vec<_>>(),
        );
        assert_eq!(
            outcome.shown(),
            expected_changes
                .iter()
                .any(|change| *change != Change::None),
        );
        assert_transcript(&actual, &expected);
        start = end;
    }
}

#[test]
fn a_run_reduced_to_one_event_keeps_its_effect_and_timestamp() {
    let mut transcript = fixture(None);
    let mut text_run = run(&[event(0, "old", 11), event(usize::MAX, "é", 12)]);
    let mut include = [false, true].into_iter();
    text_run.retain(|_| include.next().unwrap());

    let outcome = transcript.apply_text_run(&text_run);
    assert_eq!(
        outcome.event_changes().collect::<Vec<_>>(),
        [Change::Pushed]
    );
    assert_eq!(
        outcome.layout_changes().collect::<Vec<_>>(),
        [Change::Pushed]
    );
    assert!(outcome.shown());
    let entry = transcript.entries.last().unwrap();
    assert_eq!(&*entry.text, "é");
    assert_eq!(entry.time, Some(12));

    text_run.retain(|_| false);
    let mut outcome = transcript.apply_text_run(&text_run);
    assert_eq!(outcome.event_changes().next(), None);
    assert_eq!(outcome.layout_changes().next(), None);
    assert!(!outcome.shown());
    outcome.feedback_changed = true;
    assert!(outcome.shown());
}

#[test]
fn all_small_utf8_offset_sequences_match_real_reducer_at_every_partition() {
    let offsets = [0, 1, 2, 3, 5, usize::MAX];
    let texts = ["", "x", "é", "🦀"];
    let operations: Vec<_> = offsets
        .into_iter()
        .flat_map(|offset| texts.into_iter().map(move |text| (offset, text)))
        .collect();
    let mut cases = 0;
    for base in [
        None,
        Some(""),
        Some("a"),
        Some("é"),
        Some("🦀"),
        Some("é"),
        Some("aé"),
    ] {
        for &(a, x) in &operations {
            for &(b, y) in &operations {
                for &(c, z) in &operations {
                    let events = [event(a, x, 11), event(b, y, 12), event(c, z, 13)];
                    for mask in 0..4 {
                        compare_partition(base, &events, mask);
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 387_072);
}

#[test]
fn current_text_boundaries_and_shrinking_offsets_match_real_reducer() {
    for (base, operations, final_text) in [
        ("é", vec![(1, "a"), (2, "b")], "éb"),
        ("éa", vec![(0, "🦀"), (2, "b")], "🦀b"),
        ("abcdef", vec![(2, "X"), (5, "Y")], "abXY"),
    ] {
        let events: Vec<_> = operations
            .into_iter()
            .enumerate()
            .map(|(i, (offset, text))| event(offset, text, i as i64 + 11))
            .collect();
        for mask in 0..2 {
            compare_partition(Some(base), &events, mask);
        }
        let mut transcript = fixture(Some(base));
        transcript.apply_text_run(&run(&events));
        assert_eq!(&*transcript.entries[1].text, final_text);
    }
}

#[test]
fn first_nonempty_stamp_key_and_final_empty_row_survive_coalescing() {
    let events = [event(999, "", 11), event(999, "a", 12), event(0, "", 13)];
    let mut actual = fixture(None);
    let mut expected = fixture(None);
    let changes = apply_oracle(&mut expected, &events);
    let outcome = actual.apply_text_run(&run(&events));
    assert_eq!(outcome.changes.as_slice(), changes);
    assert_eq!(changes, [Change::None, Change::Pushed, Change::Updated(1)]);
    assert_transcript(&actual, &expected);
    assert_eq!(actual.entries.len(), 2);
    assert_eq!(&*actual.entries[1].text, "");
    assert_eq!(actual.entries[1].time, Some(12));
    assert!(actual.entries[1].streaming);
    assert!(!actual.entries[0].streaming);
    assert_eq!(
        outcome
            .layout_changes()
            .filter(|change| *change == Change::Pushed)
            .count(),
        1,
    );
}

#[test]
fn first_creation_uses_physical_order_not_timestamp_sorting() {
    let events = [event(0, "", 100), event(0, "a", -5), event(1, "b", -100)];
    for mask in 0..4 {
        compare_partition(None, &events, mask);
    }
    let mut transcript = fixture(None);
    transcript.apply_text_run(&run(&events));
    assert_eq!(transcript.entries[1].time, Some(-5));
}

#[test]
fn empty_noop_reopens_existing_row_without_stamping_or_stopping_other_rows() {
    let mut actual = fixture(Some("é"));
    let mut expected = fixture(Some("é"));
    let events = [event(2, "", 11), event(2, "", 12)];
    let changes = apply_oracle(&mut expected, &events);
    let outcome = actual.apply_text_run(&run(&events));
    assert_eq!(outcome.changes.as_slice(), changes);
    assert_transcript(&actual, &expected);
    assert_eq!(actual.entries[1].time, None);
    assert!(actual.entries[0].streaming);
    assert!(actual.entries[1].streaming);
    assert_eq!(&*actual.entries[1].text, "é");
}

#[test]
fn reverse_identity_lookup_preserves_nonassistant_metadata() {
    let make = || {
        let mut transcript = fixture(Some("older duplicate"));
        transcript.push_user(Some(id(100)), "latest duplicate", Vec::new());
        let entry = transcript.entries.last_mut().unwrap();
        entry.time = Some(42);
        entry.images.push(proteus_contracts::domain::ImageRef {
            id: "image".into(),
            name: "fixture.png".into(),
            mime_type: "image/png".into(),
            path: "fixture.png".into(),
        });
        entry.subagent = Some(proteus_contracts::app_protocol::AppTranscriptSubagent {
            child_thread_id: id(20).to_string(),
            session: None,
            role: "fixture".into(),
            description: Some("unchanged".into()),
            status: "running".into(),
            iterations: Some(4),
            tools: Vec::new(),
        });
        entry.tool = Some(AppTranscriptTool {
            terminal: None,
            call_id: "collision".into(),
            name: "fixture".into(),
            args: serde_json::json!({"exact": [1, 2]}),
            effective_args: Some(serde_json::json!({"resolved": true})),
            status: "done".into(),
            result: Some("result".into()),
            metadata: serde_json::json!({"keep": "all"}),
        });
        transcript
    };
    let mut actual = make();
    let mut expected = make();
    let nonassistant = actual.entries[2].clone();
    let events = [event(6, "replacement", 11), event(0, "", 12)];
    let changes = apply_oracle(&mut expected, &events);
    let outcome = actual.apply_text_run(&run(&events));
    assert_eq!(outcome.changes.as_slice(), changes);
    assert_eq!(changes, [Change::Updated(1), Change::Updated(1)]);
    assert_transcript(&actual, &expected);
    assert_eq!(actual.entries[2].role, Role::User);
    assert_eq!(actual.entries[2].time, Some(42));
    assert_entry(&actual.entries[2], &nonassistant);
    assert_eq!(&*actual.entries[1].text, "");
}

#[test]
fn foreign_root_run_is_ignored_without_consuming_identity_or_work() {
    let mut actual = fixture(None);
    let mut expected = fixture(None);
    let mut events = [event(0, "child", 11), event(5, " text", 12)];
    for event in &mut events {
        let AppServerEvent::Runtime { envelope } = event.as_mut() else {
            unreachable!();
        };
        envelope.thread_id = id(20);
    }
    let changes = apply_oracle(&mut expected, &events);
    let outcome = actual.apply_text_run(&run(&events));
    assert_eq!(outcome.changes.as_slice(), changes);
    assert_transcript(&actual, &expected);
    assert!(!outcome.shown());
    assert_eq!(actual.take_text_work(), TextWork::default());
}

#[test]
fn one_span_finish_keeps_fresh_arc_and_skips_intermediate_prefix_copy() {
    let base = "éa";
    for (operations, expected_text, prefix_bytes) in [
        ([(3, ""), (3, "")], "éa", 0),
        ([(2, ""), (2, "")], "é", 0),
        ([(0, "éa"), (3, "")], "éa", 0),
        ([(0, ""), (0, "")], "", 0),
        ([(2, "b"), (3, "")], "éb", 2),
    ] {
        let events = operations
            .into_iter()
            .enumerate()
            .map(|(i, (offset, text))| event(offset, text, i as i64 + 11))
            .collect::<Vec<_>>();
        let mut actual = fixture(Some(base));
        let mut expected = fixture(Some(base));
        let old = actual.entries[1].text.clone();
        let changes = apply_oracle(&mut expected, &events);
        let outcome = actual.apply_text_run(&run(&events));
        assert_eq!(outcome.changes.as_slice(), changes);
        assert_transcript(&actual, &expected);
        assert_eq!(&*actual.entries[1].text, expected_text);
        assert!(!std::sync::Arc::ptr_eq(&old, &actual.entries[1].text));
        assert_eq!(actual.take_text_work().prior_row_prefix_bytes, prefix_bytes);
    }
}

#[test]
fn work_budget_for_adjacent_appends() {
    let base = "p".repeat(4096);
    let text = "d".repeat(64);
    let events: Vec<_> = (0..64)
        .map(|i| event(base.len() + i * text.len(), &text, i as i64 + 11))
        .collect();
    let mut actual = fixture(Some(&base));
    let mut expected = fixture(Some(&base));
    let changes = apply_oracle(&mut expected, &events);
    let outcome = actual.apply_text_run(&run(&events));
    assert_eq!(outcome.changes.as_slice(), changes);
    assert_transcript(&actual, &expected);
    let old = expected.take_text_work();
    assert_eq!(old.delta_lookups, 64);
    assert_eq!(old.delta_row_replacements, 64);
    assert_eq!(old.prior_row_prefix_bytes, 64 * 4096 + 64 * 64 * 63 / 2);
    let work = actual.take_text_work();
    assert_eq!(
        (
            work.delta_lookups,
            work.delta_row_replacements,
            work.prior_row_prefix_bytes
        ),
        (1, 1, 4096),
        "stream-coalescing work budget: prior-row-prefix bytes/delta lookups/delta row replacements, not timing",
    );
}

#[test]
fn all_empty_absent_run_preserves_rows_keys_and_zero_delta_row_replacements() {
    let mut actual = fixture(None);
    let mut expected = fixture(None);
    let events = [
        event(0, "", 11),
        event(usize::MAX, "", 12),
        event(1, "", 13),
    ];
    let changes = apply_oracle(&mut expected, &events);
    let outcome = actual.apply_text_run(&run(&events));
    assert_eq!(outcome.changes.as_slice(), changes);
    assert_transcript(&actual, &expected);
    assert_eq!(actual.entries.len(), 1);
    assert!(actual.entries[0].streaming);
    assert!(!outcome.shown());
    assert_eq!(outcome.event_changes().count(), 0);
    assert_eq!(outcome.layout_changes().count(), 0);
    let work = actual.take_text_work();
    assert_eq!(work.prior_row_prefix_bytes, 0);
    assert_eq!(work.delta_row_replacements, 0);
}

#[test]
fn singleton_runs_preserve_real_reducer_state_and_logical_work() {
    for base in [None, Some(""), Some("é")] {
        for offset in [0, 1, 2, usize::MAX] {
            for text in ["", "x", "🦀"] {
                for thread in [10, 20] {
                    let mut actual = fixture(base);
                    let mut expected = fixture(base);
                    let mut item = event(offset, text, 11);
                    let AppServerEvent::Runtime { envelope } = item.as_mut() else {
                        unreachable!();
                    };
                    envelope.thread_id = id(thread);
                    let events = [item];
                    let changes = apply_oracle(&mut expected, &events);
                    let outcome = actual.apply_text_run(&run(&events));
                    assert_eq!(outcome.changes.as_slice(), changes);
                    assert_transcript(&actual, &expected);
                    assert_eq!(actual.take_text_work(), expected.take_text_work());
                    assert_eq!(
                        outcome.layout_changes().collect::<Vec<_>>(),
                        outcome.event_changes().collect::<Vec<_>>(),
                    );
                }
            }
        }
    }
}
