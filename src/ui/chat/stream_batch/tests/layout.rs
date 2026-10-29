//! Native-independent parity of the production row-layout instructions.
use super::*;
use crate::ui::chat::row_layout::{reconciles_rows, row_layout};
use std::collections::BTreeSet;

#[derive(Debug, Default, PartialEq, Eq)]
struct Effects {
    appends: usize,
    dirty_rows: BTreeSet<usize>,
    fresh_keys: BTreeSet<u64>,
}

impl Effects {
    fn record(&mut self, session: &Session, change: Change) {
        // Live text never enters the production snapshot row-reconciliation guard.
        assert!(!reconciles_rows(change));
        let Some(plan) = row_layout(session.transcript.entries(), change) else {
            return;
        };
        match plan.primary {
            Change::Pushed => {
                self.appends += 1;
                self.dirty_rows
                    .insert(session.transcript.entries().len() - 1);
            }
            Change::Updated(row) => {
                self.dirty_rows.insert(row);
            }
            _ => unreachable!("production row-layout plan"),
        }
        self.fresh_keys.extend(plan.fresh_key);
        if let Some(rows) = plan.chain {
            self.dirty_rows.extend(rows);
        }
    }
}

#[test]
fn text_plans_preserve_appends_fresh_keys_dirty_chains_and_no_snapshot_reconciliation() {
    assert!(reconciles_rows(Change::Spliced {
        start: 0,
        removed: 1,
        inserted: 1,
    }));
    let delta = |offset, value: &str, number| {
        event(
            Event::AssistantTextDelta {
                offset,
                message_id: id(100),
                phase: None,
                text: value.into(),
            },
            number,
        )
    };
    for case in 0..3 {
        let make = || {
            let (mut session, _) = ready();
            let rows = match case {
                0 => Vec::new(),
                1 => vec![AppTranscriptMessage {
                    message_id: Some(id(100)),
                    phase: None,
                    role: "assistant".into(),
                    text: "é".into(),
                    images: Vec::new(),
                    tool: None,
                    subagent: None,
                    streaming: false,
                }],
                _ => (0..3)
                    .map(|row| AppTranscriptMessage {
                        message_id: Some(id(if row == 1 { 100 } else { 200 + row })),
                        phase: None,
                        role: "system".into(),
                        text: "tool row".into(),
                        images: Vec::new(),
                        tool: Some(AppTranscriptTool {
                            call_id: format!("chain-{row}"),
                            name: "tool".into(),
                            args: json!({"row": row}),
                            effective_args: None,
                            status: "done".into(),
                            result: Some("unchanged".into()),
                            metadata: json!({}),
                        }),
                        subagent: None,
                        streaming: false,
                    })
                    .collect(),
            };
            session.transcript.reset(rows);
            session
        };
        let events = match case {
            0 => vec![delta(99, "", 11), delta(99, "a", 12), delta(0, "", 13)],
            1 => vec![delta(2, "", 11), delta(2, "", 12)],
            _ => vec![delta(2, "replacement", 11), delta(0, "", 12)],
        };
        let mut expected = make();
        let mut expected_effects = Effects::default();
        let mut expected_changes = Vec::new();
        let mut expected_updates = Vec::new();
        for event in &events {
            let (changes, updates) = oracle(&mut expected, &[event.clone()]);
            for &change in &changes {
                expected_effects.record(&expected, change);
            }
            expected_changes.extend(changes);
            expected_updates.extend(updates);
        }

        let mut actual = make();
        let outcome = actual.apply_text_run(run(events));
        let mut actual_effects = Effects::default();
        for change in outcome.layout_changes() {
            actual_effects.record(&actual, change);
        }
        assert_eq!(actual_effects, expected_effects);
        assert_eq!(observed_rows(&actual), observed_rows(&expected));
        assert_eq!(
            outcome.event_changes().collect::<Vec<_>>(),
            expected_changes
        );
        assert_eq!(
            outcome
                .event_changes()
                .filter_map(|change| projection(&actual, change))
                .collect::<Vec<_>>(),
            expected_updates,
        );
        if case == 0 {
            assert_eq!(actual_effects.appends, 1);
            assert_eq!(actual_effects.fresh_keys.len(), 1);
        } else {
            assert_eq!(actual_effects.appends, 0);
            assert!(actual_effects.fresh_keys.is_empty());
        }
        if case == 2 {
            assert_eq!(actual_effects.dirty_rows, BTreeSet::from([0, 1, 2]));
            assert_eq!(
                row_layout(actual.transcript.entries(), Change::Updated(1))
                    .unwrap()
                    .chain,
                Some(0..3),
            );
        }
    }
}
