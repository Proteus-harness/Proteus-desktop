use super::*;
use crate::ui::settings::SettingsContext;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderPlugin, ConfigBuilderPluginExport, ConfigBuilderSnapshot, ConfigBuilderTool,
};
use serde_json::json;

#[::core::prelude::v1::test]
fn multiline_string_array_uses_the_json_editor_instead_of_single_line_tags() {
    let field = ConfigField::new(
        "notes",
        "Notes",
        "",
        ConfigValueSchema::Array {
            items: Box::new(ConfigValueSchema::String {
                multiline: true,
                secret: false,
            }),
        },
    );
    assert!(!is_tag_field(&field));
    assert!(matches!(
        crate::settings::field_kind(&field),
        crate::settings::FieldKind::Text(crate::settings::TextFormat::StringArrayJson)
    ));
    let format = crate::settings::TextFormat::StringArrayJson;
    let items = json!(["first\nsecond", "", "first\nsecond"]);
    let text = crate::settings::format_value(format, &items);
    assert_eq!(crate::settings::parse_text(format, &text), Ok(Some(items)));
    for bad in ["42", "[1]", r#"["ok", null]"#] {
        assert!(crate::settings::parse_text(format, bad).is_err());
    }
}

#[::core::prelude::v1::test]
fn literal_enum_key_takes_priority_over_another_choices_title() {
    use proteus_contracts::domain::ConfigChoice;
    let field = ConfigField::new(
        "values",
        "Values",
        "",
        ConfigValueSchema::Array {
            items: Box::new(ConfigValueSchema::Enum {
                options: vec![
                    ConfigChoice {
                        value: json!("first"),
                        title: "second".into(),
                    },
                    ConfigChoice {
                        value: json!("second"),
                        title: "Other".into(),
                    },
                ],
            }),
        },
    );
    assert_eq!(added_value(&field, "second"), Some(json!("second")));
    assert_eq!(added_value(&field, "Other"), Some(json!("second")));
    assert_eq!(added_value(&field, "unknown"), Some(json!("unknown")));
}

fn with_view(value: Option<Value>, run: impl FnOnce(Entity<SettingsView>, &mut App)) {
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
        let mut config = json!({"sibling":true});
        if let Some(value) = value {
            config["values"] = value;
        }
        view.accept(ConfigBuilderSnapshot {
            writable: true,
            permission_mode: "normal".into(),
            module_config: serde_json::from_value(json!({"context":{"ordered":config}})).unwrap(),
            ..Default::default()
        });
    });
    cx.update(|cx| run(view, cx));
}

fn path() -> FieldPath {
    FieldPath::new("context", "ordered", "values")
}
fn field() -> ConfigField {
    ConfigField::new("values", "Values", "", ConfigValueSchema::strings())
}
fn current(view: &SettingsView, field: &ConfigField) -> Vec<Value> {
    array(view.draft.value(&path()).or(field.default.as_ref()))
        .unwrap()
        .to_vec()
}
fn apply(view: &mut SettingsView, field: &ConfigField, edit: Edit, cx: &mut Context<SettingsView>) {
    let expected = current(view, field);
    assert!(view.apply_tag_edit(&path(), field, &expected, edit, cx));
}
fn clean(view: &SettingsView) {
    assert_eq!(view.draft, view.baseline);
    assert!(!view.dirty());
    assert!(view.draft.changes(&view.baseline).module_config.is_empty());
}

#[::core::prelude::v1::test]
fn tag_handlers_preserve_duplicates_unknown_order_and_exact_raw_baseline() {
    let original = json!([" custom * ", "known", "known", ""]);
    with_view(Some(original.clone()), |view, cx| {
        view.update(cx, |view, cx| {
            let field = field();
            apply(view, &field, Edit::Add(json!("новое значение")), cx);
            assert_eq!(
                current(view, &field),
                json!([" custom * ", "known", "known", "", "новое значение"])
                    .as_array()
                    .unwrap()
                    .clone()
            );
            apply(view, &field, Edit::Remove(4), cx);
            clean(view);
            apply(
                view,
                &field,
                Edit::MoveBefore {
                    from: 0,
                    before: None,
                },
                cx,
            );
            assert_eq!(
                current(view, &field),
                json!(["known", "known", "", " custom * "])
                    .as_array()
                    .unwrap()
                    .clone()
            );
            apply(
                view,
                &field,
                Edit::MoveBefore {
                    from: 3,
                    before: Some(0),
                },
                cx,
            );
            clean(view);
            apply(view, &field, Edit::Remove(1), cx);
            assert_eq!(
                current(view, &field),
                json!([" custom * ", "known", ""])
                    .as_array()
                    .unwrap()
                    .clone()
            );
            apply(view, &field, Edit::Add(json!("known")), cx);
            apply(
                view,
                &field,
                Edit::MoveBefore {
                    from: 3,
                    before: Some(1),
                },
                cx,
            );
            clean(view);
            assert_eq!(view.draft.value(&path()), Some(&original));
            assert_eq!(
                view.draft
                    .value(&FieldPath::new("context", "ordered", "sibling")),
                Some(&json!(true))
            );
        })
    });
}

#[::core::prelude::v1::test]
fn default_tag_undo_restores_omission_and_stale_reorder_never_overwrites_newer_edits() {
    with_view(None, |view, cx| {
        view.update(cx, |view, cx| {
            let field = field().with_default(json!(["inherited"]));
            let expected = current(view, &field);
            apply(view, &field, Edit::Add(json!("new")), cx);
            assert!(!view.apply_tag_edit(&path(), &field, &expected, Edit::Remove(0), cx));
            assert_eq!(
                current(view, &field),
                json!(["inherited", "new"]).as_array().unwrap().clone()
            );
            apply(view, &field, Edit::Remove(1), cx);
            clean(view);
            assert_eq!(view.draft.value(&path()), None);
        })
    });
}

#[::core::prelude::v1::test]
fn suggestions_are_schema_owned_and_never_rewrite_unknown_saved_values() {
    let tool = |name: &str| ConfigBuilderTool {
        name: name.into(),
        ..Default::default()
    };
    let export = |slot: &str, id: &str| ConfigBuilderPluginExport {
        slot: slot.into(),
        id: id.into(),
        active: true,
        description: None,
        config_schema: None,
    };
    let snapshot = ConfigBuilderSnapshot {
        tools: vec![tool("shell"), tool("patch"), tool("shell")],
        plugins: vec![ConfigBuilderPlugin {
            id: "fixture".into(),
            command: "synthetic".into(),
            description: None,
            icon: None,
            tool_packs: Vec::new(),
            exports: vec![
                export("custom_slot", "one"),
                export("custom_slot", "two"),
                export("other", "wrong"),
                export("custom_slot", "one"),
            ],
        }],
        ..Default::default()
    };
    let values = |field: &ConfigField| {
        suggest::visible(field, Some(&snapshot), &[], "")
            .into_iter()
            .map(|item| item.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&field().suggest_tools()), ["shell", "patch"]);
    assert_eq!(
        values(&field().suggest_exports("custom_slot")),
        ["one", "two"]
    );
    assert!(values(&field()).is_empty());
    assert_eq!(
        values(&field().with_default(json!(["module_builtin", "module_builtin"]))),
        ["module_builtin"]
    );
    assert_eq!(
        added_value(&field(), " custom * "),
        Some(json!(" custom * "))
    );
    let choices = ConfigField::new(
        "values",
        "Values",
        "",
        ConfigValueSchema::Array {
            items: Box::new(ConfigValueSchema::choices(&[
                ("a", "First"),
                ("b", "Second"),
            ])),
        },
    );
    assert!(is_tag_field(&choices));
    assert_eq!(values(&choices), ["a", "b"]);
    assert_eq!(added_value(&choices, "Second"), Some(json!("b")));
    assert_eq!(added_value(&choices, "unknown"), Some(json!("unknown")));
    let raw = json!(["unknown", "a", "a", 7]);
    let ids = item_ids(raw.as_array().unwrap());
    assert_eq!(
        ids.iter().collect::<std::collections::HashSet<_>>().len(),
        4
    );
    let next = edited(
        raw.as_array().unwrap(),
        Edit::MoveBefore {
            from: 2,
            before: Some(0),
        },
    )
    .unwrap();
    assert_eq!(
        next,
        json!(["a", "unknown", "a", 7]).as_array().unwrap().clone()
    );
    with_view(Some(json!({"not":"an array"})), |view, cx| {
        view.update(cx, |view, cx| {
            assert!(!view.apply_tag_edit(
                &path(),
                &field(),
                &[],
                Edit::Add(json!("replacement")),
                cx
            ));
            clean(view);
        })
    });
}
