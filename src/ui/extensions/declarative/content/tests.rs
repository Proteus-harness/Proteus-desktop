use super::{Content, updated_content};
use serde_json::json;

fn current(revision: Option<u64>) -> Content {
    Content {
        language: "fixture".into(),
        source: "original source\n".into(),
        context: json!({"languages": ["fixture"], "custom": {"preserve": true}}),
        source_revision: revision,
    }
}

#[test]
fn unchanged_borrowed_source_keeps_the_current_snapshot_and_revision() {
    let current = current(Some(7));
    let source_pointer = current.source.as_ptr();
    assert!(
        updated_content(
            &current,
            &current.language,
            &current.source,
            current.context.clone(),
        )
        .is_none()
    );
    assert_eq!(current.source.as_ptr(), source_pointer);
    assert_eq!(current.source_revision, Some(7));
}

#[test]
fn changed_source_is_owned_once_then_shared_without_a_payload_clone() {
    let current = current(Some(7));
    let source = "αβ\n😀\0tail\r\n".repeat(4096);
    let next = updated_content(&current, "fixture", &source, current.context.clone()).unwrap();
    let runtime = std::sync::Arc::clone(&next);
    assert!(std::sync::Arc::ptr_eq(&next, &runtime));
    assert_eq!(next.source.as_ptr(), runtime.source.as_ptr());
    assert_eq!(next.source, source);
    assert_eq!(next.source_revision, Some(8));
    assert_eq!(next.context, current.context);
    assert_eq!(current.source, "original source\n");
    assert_eq!(
        serde_json::to_value(next.as_ref()).unwrap(),
        json!({
            "language": "fixture",
            "source": source,
            "context": current.context,
            "source_revision": 8,
        })
    );
}

#[test]
fn language_and_context_changes_still_advance_the_revision() {
    let current = current(Some(7));
    let language =
        updated_content(&current, "other", &current.source, current.context.clone()).unwrap();
    assert_eq!(language.source_revision, Some(8));
    assert_eq!(language.language, "other");
    assert_eq!(language.source, current.source);

    let context = json!({"languages": ["fixture", "new"], "custom": {"preserve": true}});
    let next = updated_content(&current, "fixture", &current.source, context.clone()).unwrap();
    assert_eq!(next.source_revision, Some(8));
    assert_eq!(next.context, context);
    assert_eq!(next.source, current.source);
}

#[test]
fn absent_and_wrapping_revisions_keep_existing_semantics() {
    for (before, after) in [(None, Some(1)), (Some(u64::MAX), Some(0))] {
        let current = current(before);
        let next = updated_content(&current, "fixture", "", current.context.clone()).unwrap();
        assert_eq!(next.source_revision, after);
        assert!(next.source.is_empty());
    }
}
