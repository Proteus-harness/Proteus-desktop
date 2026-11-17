//! Exercise the handlers used by schema controls, including their saved raw
//! representation. A clean indicator alone would not protect the save request.

use super::*;
use crate::ui::settings::SettingsContext;
use proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot;
use proteus_contracts::domain::ConfigValueSchema;
use serde_json::json;

fn with_view(config: Value, run: impl FnOnce(Entity<SettingsView>, &mut App)) {
    let mut cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = cx.add_empty_window();
    let view = window.update(|window, cx| {
        cx.new(|cx| {
            SettingsView::new(
                WeakEntity::new_invalid(),
                SettingsContext::for_chat(None, || None),
                window,
                cx,
            )
        })
    });
    view.update(&mut cx, |view, _| {
        view.accept(ConfigBuilderSnapshot {
            writable: true,
            permission_mode: "normal".into(),
            module_config: serde_json::from_value(json!({"tool":{"codex_patch":config}})).unwrap(),
            ..Default::default()
        });
    });
    cx.update(|cx| run(view, cx));
}

fn switch(view: &Entity<SettingsView>, path: FieldPath) -> FieldSpec {
    FieldSpec {
        path,
        field: ConfigField::new(
            "freeform",
            "Свободный формат Codex",
            "",
            ConfigValueSchema::Boolean {},
        )
        .with_default(false),
        value: None,
        view: view.downgrade(),
    }
}

fn assert_clean(view: &SettingsView) {
    assert_eq!(view.draft, view.baseline);
    assert!(!view.dirty());
    assert!(!view.has_unsaved_changes());
    assert!(view.draft.areas(&view.baseline).is_empty());
    assert!(view.draft.changes(&view.baseline).module_config.is_empty());
}

#[::core::prelude::v1::test]
fn reset_restores_the_saved_list_order_and_keeps_other_edits() {
    let saved = json!(["read", "shell", "read"]);
    with_view(
        json!({"input":{"tools":saved,"other":false}}),
        |view, cx| {
            let group = FieldPath::new("tool", "codex_patch", "input");
            let tools = group.child("tools");
            let other = group.child("other");
            view.update(cx, |view, cx| {
                view.edit_field(&tools, Some(json!([])), None, cx);
                view.edit_field(&other, Some(json!(true)), None, cx);
                view.reset_field(&tools, cx);
                assert_eq!(view.draft.value(&tools), Some(&saved));
                assert_eq!(view.draft.value(&other), Some(&json!(true)));
                view.reset_field(&group, cx);
                assert_clean(view);
            });
        },
    );
}

#[::core::prelude::v1::test]
fn switch_revert_restores_explicit_and_omitted_defaults_without_reverting_siblings() {
    for explicit in [false, true] {
        let mut config = json!({"reject_self_move":false});
        if explicit {
            config["freeform"] = json!(false);
        }
        with_view(config, |view, cx| {
            let spec = switch(&view, FieldPath::new("tool", "codex_patch", "freeform"));
            let sibling = FieldPath::new("tool", "codex_patch", "reject_self_move");
            spec.set(Some(json!(true)), cx);
            view.update(cx, |view, _| {
                assert!(view.dirty());
                assert_eq!(view.draft.value(&sibling), Some(&json!(false)));
            });
            spec.set(Some(json!(false)), cx);
            view.update(cx, |view, cx| {
                assert_clean(view);
                view.update_draft(cx, |draft| draft.set_value(&sibling, Some(json!(true))));
            });
            spec.set(Some(json!(true)), cx);
            spec.set(Some(json!(false)), cx);
            view.update(cx, |view, _| {
                assert_eq!(
                    view.draft.value(&spec.path),
                    view.baseline.value(&spec.path)
                );
                assert_eq!(view.draft.value(&sibling), Some(&json!(true)));
                assert!(view.dirty());
                assert_eq!(view.draft.areas(&view.baseline).len(), 1);
                let saved = view.draft.changes(&view.baseline);
                assert_eq!(
                    saved.module_config["tool"]["codex_patch"]["reject_self_move"],
                    true
                );
            });
        });
    }
}

#[::core::prelude::v1::test]
fn switch_revert_preserves_saved_empty_ancestors_and_prunes_new_ones() {
    for config in [
        json!({"reject_self_move":false}),
        json!({"options":{},"reject_self_move":false}),
        json!({"options":{"nested":{}},"reject_self_move":false}),
    ] {
        with_view(config, |view, cx| {
            let spec = switch(
                &view,
                FieldPath::new("tool", "codex_patch", "options")
                    .child("nested")
                    .child("freeform"),
            );
            spec.set(Some(json!(true)), cx);
            spec.set(Some(json!(false)), cx);
            view.update(cx, |view, _| assert_clean(view));
        });
    }
}

#[::core::prelude::v1::test]
fn text_revert_uses_the_same_raw_baseline_rules_as_switches() {
    for explicit in [false, true] {
        let mut config = json!({"reject_self_move":false});
        if explicit {
            config["limit"] = json!(0);
        }
        with_view(config, |view, cx| {
            view.update(cx, |view, cx| {
                let path = FieldPath::new("tool", "codex_patch", "limit");
                let sibling = FieldPath::new("tool", "codex_patch", "reject_self_move");
                let format = TextFormat::Integer {
                    minimum: None,
                    maximum: None,
                };
                view.text_changed(&path, format, Some(&json!(0)), "7", cx);
                assert!(view.dirty());
                view.text_changed(&path, format, Some(&json!(0)), "0", cx);
                assert_clean(view);
                view.update_draft(cx, |draft| draft.set_value(&sibling, Some(json!(true))));
                view.text_changed(&path, format, Some(&json!(0)), "7", cx);
                view.text_changed(&path, format, Some(&json!(0)), "0", cx);
                assert_eq!(view.draft.value(&path), view.baseline.value(&path));
                assert_eq!(view.draft.value(&sibling), Some(&json!(true)));
                assert!(view.dirty());
                assert!(!view.draft.changes(&view.baseline).module_config.is_empty());
            });
        });
    }
}
