use super::*;
use proteus_contracts::app_protocol::AppTranscriptMessage;

fn message(role: &str, text: &str) -> AppTranscriptMessage {
    AppTranscriptMessage {
        message_id: None,
        timestamp_ms: None,
        phase: None,
        role: role.into(),
        text: text.into(),
        images: vec![],
        tool: None,
        subagent: None,
        streaming: false,
    }
}

fn cursor(before: usize) -> AppTranscriptCursor {
    AppTranscriptCursor {
        revision: "frozen".into(),
        before,
        base: None,
    }
}

fn page(start: usize, messages: Vec<AppTranscriptMessage>) -> AppTranscriptPage {
    AppTranscriptPage {
        revision: "frozen".into(),
        start,
        total: 8,
        messages,
        base: None,
    }
}

#[test]
fn core_copy_pages_only_to_user_preserving_reply_order_and_original_whitespace() {
    let mut parts = Parts::default();
    parts.push(" final\n".into()).unwrap();
    let mut requested = vec![];
    read_core(&mut parts, cursor(6), |cursor| {
        requested.push(cursor.before);
        Ok(match cursor.before {
            6 => page(
                4,
                vec![
                    message("assistant", "middle"),
                    message("system", "tool output"),
                ],
            ),
            4 => page(
                0,
                vec![
                    message("assistant", "previous turn"),
                    message("user", "question"),
                    message("assistant", " first\n"),
                    message("system", "reasoning"),
                ],
            ),
            _ => panic!("copy must stop at the preceding user"),
        })
    })
    .unwrap();
    assert_eq!(requested, vec![6, 4]);
    assert_eq!(parts.finish().unwrap(), " first\n\n\nmiddle\n\n final\n");
}

#[test]
fn core_beginning_without_user_and_unknown_dates_is_still_complete() {
    let mut parts = Parts::default();
    read_core(&mut parts, cursor(1), |_| {
        Ok(page(0, vec![message("assistant", "initial response")]))
    })
    .unwrap();
    assert_eq!(parts.finish().unwrap(), "initial response");
}

#[test]
fn core_rejects_changed_revision_gaps_and_nonprogress_instead_of_partial_copy() {
    for failure in ["revision", "gap", "empty", "total"] {
        let mut parts = Parts::default();
        parts.push("tail".into()).unwrap();
        let result = read_core(&mut parts, cursor(2), |_| {
            let mut page = page(1, vec![message("assistant", "older")]);
            match failure {
                "revision" => page.revision = "different".into(),
                "gap" => page.start = 0,
                "empty" => page.messages.clear(),
                "total" => page.total = 1,
                _ => unreachable!(),
            }
            Ok(page)
        });
        assert!(result.is_err(), "accepted {failure}");
    }
}

fn item(turn: &str, id: &str, kind: ItemKind) -> Item {
    Item {
        turn_id: turn.into(),
        id: id.into(),
        kind,
        status: "completed".into(),
        timestamp_ms: None,
    }
}

fn assistant(turn: &str, id: &str, text: &str) -> Item {
    item(turn, id, ItemKind::Assistant { text: text.into() })
}

fn anchor() -> Anchor {
    Anchor {
        turn: "turn/opaque".into(),
        item: "anchor:opaque".into(),
        role: Role::Assistant,
        text: "tail".into(),
    }
}

fn seen() -> HashSet<String> {
    HashSet::from(["anchor:opaque".into()])
}

#[test]
fn codex_resolves_opaque_anchor_then_reads_only_the_same_turn() {
    let mut parts = Parts::default();
    parts.push("tail".into()).unwrap();
    let mut requested = vec![];
    read_codex(
        &mut parts,
        "turn/opaque",
        anchor(),
        None,
        seen(),
        |cursor| {
            requested.push(cursor.map(str::to_owned));
            Ok(match cursor {
                None => Page {
                    data: vec![assistant("new turn", "new", "not copied")],
                    next_cursor: Some("seek:opaque".into()),
                },
                Some("seek:opaque") => Page {
                    data: vec![
                        assistant("turn/opaque", "middle", "middle"),
                        assistant("turn/opaque", "anchor:opaque", "tail"),
                        assistant("new turn", "newer", "not copied"),
                    ],
                    next_cursor: Some("older:opaque".into()),
                },
                Some("older:opaque") => Page {
                    data: vec![
                        assistant("previous turn", "previous", "not copied"),
                        assistant("turn/opaque", "first", " first\n"),
                        item(
                            "turn/opaque",
                            "reason",
                            ItemKind::Reasoning {
                                text: "not copied".into(),
                            },
                        ),
                    ],
                    next_cursor: Some("must-not-read".into()),
                },
                _ => panic!("read beyond the copied turn"),
            })
        },
    )
    .unwrap();
    assert_eq!(
        requested,
        vec![
            None,
            Some("seek:opaque".into()),
            Some("older:opaque".into())
        ]
    );
    assert_eq!(parts.finish().unwrap(), " first\n\n\nmiddle\n\ntail");
}

#[test]
fn codex_rejects_missing_changed_anchor_and_cursor_cycles() {
    for failure in ["missing", "changed", "cycle"] {
        let result = read_codex(
            &mut Parts::default(),
            "turn/opaque",
            anchor(),
            None,
            seen(),
            |_| {
                Ok(match failure {
                    "missing" => Page {
                        data: vec![],
                        next_cursor: None,
                    },
                    "changed" => Page {
                        data: vec![assistant("turn/opaque", "anchor:opaque", "changed")],
                        next_cursor: None,
                    },
                    "cycle" => Page {
                        data: vec![],
                        next_cursor: Some("same".into()),
                    },
                    _ => unreachable!(),
                })
            },
        );
        assert!(result.is_err(), "accepted {failure}");
    }
}

#[test]
fn codex_rejects_overlap_with_already_copied_rows() {
    let mut calls = 0;
    let result = read_codex(
        &mut Parts::default(),
        "turn/opaque",
        anchor(),
        None,
        seen(),
        |_| {
            calls += 1;
            Ok(Page {
                data: vec![assistant("turn/opaque", "anchor:opaque", "tail")],
                next_cursor: Some(format!("page:{calls}")),
            })
        },
    );
    assert!(result.is_err());
    assert_eq!(calls, 2);
}

#[test]
fn copied_text_budget_counts_separators_and_never_truncates() {
    let text: Arc<str> = "x".repeat(MAX_BYTES - 3).into();
    let mut parts = Parts::default();
    parts.push(text.clone()).unwrap();
    assert!(parts.push("é".into()).is_err());
    assert_eq!(parts.finish().unwrap().as_str(), text.as_ref());
    assert!(Parts::default().finish().is_err());
}
