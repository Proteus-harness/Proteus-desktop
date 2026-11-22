use super::*;
use proteus_contracts::domain::ImageRef;
use serde_json::json;
use uuid::Uuid;

fn row(role: &str, id: Option<u128>, text: &str) -> AppTranscriptMessage {
    AppTranscriptMessage {
        message_id: id.map(Uuid::from_u128),
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

#[test]
fn page_reuses_first_live_text_without_changing_identity_fields_or_next_key() {
    let mut live = Transcript::default();
    let first = row("assistant", Some(7), "overlap 🦀");
    let later = row("assistant", Some(7), "different duplicate");
    let reasoning = row("reasoning", Some(7), "overlap 🦀");
    let mut tool = row("system", None, "");
    tool.tool = Some(AppTranscriptTool {
        terminal: None,
        call_id: "same-id".into(),
        name: "fixture".into(),
        args: json!({"data": "original"}),
        effective_args: None,
        status: "done".into(),
        result: None,
        metadata: Value::Null,
    });
    let mut child = row("system", None, "");
    child.subagent = Some(AppTranscriptSubagent {
        child_thread_id: "same-id".into(),
        session: None,
        role: "runner".into(),
        description: None,
        status: "running".into(),
        iterations: None,
        tools: vec![],
    });
    let anonymous = row("system", None, "anonymous");
    let mut originals = vec![
        first.clone(),
        later.clone(),
        reasoning.clone(),
        tool.clone(),
        child.clone(),
        anonymous.clone(),
    ];
    for (index, original) in originals.iter_mut().enumerate() {
        original.timestamp_ms = Some((index as i64 + 1) * 11);
    }
    live.prepend(originals);

    // Both trees share the sequence counter when cloning a transcript.
    let mut window = live.clone();
    window.remove(0..window.entries().len());
    window.prepend(vec![row("user", None, "anchor")]);
    let mut first = first;
    first.timestamp_ms = Some(101);
    first.streaming = true;
    first.images.push(ImageRef {
        id: "fixture".into(),
        name: "synthetic.png".into(),
        mime_type: "image/png".into(),
        path: "synthetic.png".into(),
    });
    tool.tool.as_mut().unwrap().args = json!({"data": ["new", 2]});
    child.subagent.as_mut().unwrap().status = "done".into();
    let missing = row("assistant", Some(8), "overlap 🦀");
    assert_eq!(
        window.append_page(
            vec![first, later, reasoning, tool, child, anonymous, missing],
            &live
        ),
        Change::Spliced {
            start: 1,
            removed: 0,
            inserted: 7
        }
    );
    let rows = window.entries();
    assert_eq!(
        rows.iter().map(|row| row.key).collect::<Vec<_>>(),
        [7, 1, 1, 3, 4, 5, 13, 14]
    );
    assert_eq!(
        rows.iter().map(|row| row.time).collect::<Vec<_>>(),
        [
            None,
            Some(101),
            Some(11),
            Some(33),
            Some(44),
            Some(55),
            None,
            None
        ]
    );
    for (inserted, original) in [(1, 0), (3, 2), (4, 3), (5, 4)] {
        assert!(Arc::ptr_eq(
            &rows[inserted].text,
            &live.entries()[original].text
        ));
    }
    assert_eq!(&*rows[2].text, "different duplicate");
    assert!(!Arc::ptr_eq(&rows[2].text, &live.entries()[1].text));
    assert!(!Arc::ptr_eq(&rows[6].text, &live.entries()[5].text));
    assert!(!Arc::ptr_eq(&rows[7].text, &live.entries()[0].text));
    assert_eq!(rows[1].images[0].name, "synthetic.png");
    assert!(rows[1].streaming);
    assert_eq!(
        rows[4].tool.as_ref().unwrap().args,
        json!({"data": ["new", 2]})
    );
    assert_eq!(rows[5].subagent.as_ref().unwrap().status, "done");
    window.prepend(vec![row("user", None, "next")]);
    assert_eq!(window.entries()[0].key, 15);
}
