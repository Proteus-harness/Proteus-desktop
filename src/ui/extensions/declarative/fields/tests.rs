//! Retained native inputs must follow the current declaration without replacing drafts.
use super::*;
use crate::declarative::{manifest::Installed, protocol::SurfaceKind};
use crate::ui::extensions::{Host, host::Lifetime};
use serde_json::json;

#[::core::prelude::v1::test]
fn input_rebind_keeps_the_native_draft_and_preview_separate_from_disclosures() {
    let mut app = ::gpui::TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = app.add_empty_window();
    window.update(|window, cx| {
        let installed = Box::leak(Box::new(Installed {
            dir: "/synthetic-retained-input".into(),
            revision: 0,
            manifest: serde_json::from_value(json!({
                "protocol":4, "id":"input-fixture", "title":"Fixture",
                "command":["never-start"], "surfaces":{"page":{}}
            }))
            .unwrap(),
        }));
        let host =
            Host::new(WeakEntity::new_invalid(), "input-fixture").isolated(Lifetime::Preview);
        let runtime =
            cx.new(|cx| super::super::runtime::Runtime::new(installed, host.clone(), window, cx));
        runtime.update(cx, |runtime, _| runtime.error = Some("hold startup".into()));
        let view =
            cx.new(|cx| DeclarativeView::new(runtime, host, SurfaceKind::Page, None, window, cx));
        let input = |action: &str, submit: Option<&str>, lines: usize| {
            serde_json::from_value::<Node>(json!({
                "type":"input", "id":"note", "value":"declared", "placeholder":"Note",
                "action":action, "submit":submit, "lines":lines
            }))
            .unwrap()
        };
        view.update(cx, |view, cx| {
            view.prepare_fields(&input("old-change", Some("old-submit"), 3), window, cx);
            let Text::Area(state) = &view.fields.inputs["note"].state else {
                panic!("expected textarea")
            };
            let state = state.clone();
            let native = state.entity_id();
            state.update(cx, |state, cx| state.set_value("unsent draft", window, cx));
            view.fields
                .disclosures
                .insert("input-preview:note".into(), false);
            view.fields.set_input_preview("note", true);
            view.prepare_fields(&input("new-change", Some("new-submit"), 7), window, cx);
            let retained = &view.fields.inputs["note"];
            assert!(matches!(&retained.state, Text::Area(state) if state.entity_id() == native));
            assert_eq!(retained.state.value(cx).as_ref(), "unsent draft");
            assert_eq!(retained.lines, 7);
            assert_eq!(
                retained.action_for("unsent draft", &InputEvent::Change),
                Some("new-change")
            );
            assert_eq!(retained.action_for("declared", &InputEvent::Change), None);
            let enter = InputEvent::PressEnter {
                secondary: true,
                shift: false,
            };
            assert_eq!(
                retained.action_for("unsent draft", &enter),
                Some("new-submit")
            );
            assert!(retained.preview_open);
            assert!(!view.fields.disclosures["input-preview:note"]);
            view.prepare_fields(&input("new-change", None, 7), window, cx);
            assert_eq!(
                view.fields.inputs["note"].action_for("unsent draft", &enter),
                None
            );
        });
    });
}

#[::core::prelude::v1::test]
fn retained_widget_actions_send_the_current_binding() {
    let app = ::gpui::TestAppContext::single();
    let sent = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = sent.clone();
    let mut action = WidgetAction::new("old", move |name, value, _| {
        observed.borrow_mut().push((name, value))
    });
    app.update(|cx| {
        action.row("row", "preview", cx);
        action.rebind("current");
        action.row("row", "open", cx);
        action.emit(json!("node"), cx);
    });
    assert_eq!(
        *sent.borrow(),
        vec![
            ("old".into(), json!({"id":"row","event":"preview"})),
            ("current".into(), json!({"id":"row","event":"open"})),
            ("current".into(), json!("node")),
        ]
    );
}
