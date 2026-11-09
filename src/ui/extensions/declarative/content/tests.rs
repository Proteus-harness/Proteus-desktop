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

#[::core::prelude::v1::test]
fn native_rich_text_prepares_behind_fallback_and_keeps_ready_replacements_visible() {
    use super::*;
    use crate::declarative::{manifest::Installed, widget::Node};
    use crate::ui::extensions::{Host, host::Lifetime};

    let mut app = ::gpui::TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = app.add_empty_window();
    let (content, runtime) = window.update(|window, cx| {
        let installed = Box::leak(Box::new(Installed {
            dir: "/synthetic-readiness".into(),
            revision: 0,
            manifest: serde_json::from_value(json!({
                "protocol":4, "id":"readiness-fixture", "title":"Fixture",
                "command":["never-start"], "surfaces":{"content":{"languages":["fixture"]}}
            }))
            .unwrap(),
        }));
        let host =
            Host::new(WeakEntity::new_invalid(), "readiness-fixture").isolated(Lifetime::Preview);
        let runtime =
            cx.new(|cx| super::super::runtime::Runtime::new(installed, host.clone(), window, cx));
        // Attach the native surface without starting a renderer process. The
        // test supplies accepted protocol trees directly, not a fake parser.
        runtime.update(cx, |runtime, _| runtime.error = Some("hold startup".into()));
        let content = cx.new(|cx| {
            let source = Arc::new(Content {
                language: "fixture".into(),
                source: "raw source remains visible while native HTML is pending".into(),
                context: serde_json::Value::Null,
                source_revision: Some(0),
            });
            let rendered = cx.new(|cx| {
                DeclarativeView::with_content(
                    runtime.clone(),
                    host,
                    SurfaceKind::Content,
                    Some("fixture".into()),
                    Some(source.clone()),
                    window,
                    cx,
                )
            });
            ContentView {
                _rendered: cx.observe(&rendered, |_, _, cx| cx.notify()),
                _resize: cx.subscribe(&rendered, |_, _, _: &super::super::view::WillResize, cx| {
                    cx.emit(WillResize)
                }),
                rendered,
                content: source,
                extension: "readiness-fixture",
                lifetime: Lifetime::Preview,
                show_source: false,
                body: false,
            }
        });
        (content, runtime)
    });
    runtime.update(window, |runtime, _| runtime.error = None);
    let html = format!("<p>{}</p>", "rendered content ".repeat(700));
    let show = |content: &mut ContentView, id: &str, html: &str, cx: &mut Context<ContentView>| {
        content.rendered.update(cx, |view, cx| {
            view.show(
                Node::RichText {
                    id: id.into(),
                    html: html.into(),
                    inlines: vec![],
                },
                0,
                Default::default(),
                cx,
            );
        });
    };
    window.update(|window, cx| {
        content.update(cx, |content, cx| {
            show(content, "initial", &html, cx);
            assert!(!content.rendered.read(cx).ready(cx));
            let _ = content.render(window, cx);
            let view = content.rendered.read(cx);
            assert!(
                view.fields.rich_text.contains_key("initial"),
                "preparation starts before a preview is mounted"
            );
            assert!(
                !view.ready(cx),
                "an unparsed native document must keep the source fallback"
            );
        });
    });
    window.run_until_parked();
    assert!(content.read_with(window, |content, cx| content.rendered.read(cx).ready(cx)));

    // A later full replacement keeps the already committed visual document,
    // rather than switching back to the much taller raw source while parsing.
    window.update(|window, cx| {
        content.update(cx, |content, cx| {
            show(content, "initial", &html.repeat(2), cx);
            let _ = content.render(window, cx);
            assert!(content.rendered.read(cx).ready(cx));

            content.show_source = true;
            show(content, "next-tree", &html, cx);
            let _ = content.render(window, cx);
            assert!(
                !content
                    .rendered
                    .read(cx)
                    .fields
                    .rich_text
                    .contains_key("next-tree"),
                "forced source stays lazy"
            );
            // This is the source toggle's exit action, allowed even when the
            // newly accepted tree has not prepared a native document yet.
            content.show_source = false;
            let _ = content.render(window, cx);
            assert!(
                content
                    .rendered
                    .read(cx)
                    .fields
                    .rich_text
                    .contains_key("next-tree")
            );
            assert!(!content.rendered.read(cx).ready(cx));
        });
    });
    window.run_until_parked();
    assert!(content.read_with(window, |content, cx| content.rendered.read(cx).ready(cx)));
}
