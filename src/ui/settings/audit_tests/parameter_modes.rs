use super::*;
use crate::ui::settings::SettingsContext;
use proteus_contracts::app_protocol::config_builder::{ConfigBuilderModule, ConfigBuilderSnapshot};
use proteus_contracts::domain::{ConfigValueSchema, ModuleConfigSchema};
use serde_json::json;

fn with_view(run: impl FnOnce(&mut SettingsView, &mut Window, &mut Context<SettingsView>)) {
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
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(ConfigBuilderSnapshot {
                writable: true,
                permission_mode: "normal".into(),
                model_modules: vec![ConfigBuilderModule {
                    id: "fixture".into(),
                    slot: "model".into(),
                    active: true,
                    config_schema: Some(ModuleConfigSchema {
                        fields: vec![ConfigField::new(
                            "limit",
                            "Limit",
                            "",
                            ConfigValueSchema::integer(0),
                        )],
                    }),
                    ..Default::default()
                }],
                module_config: serde_json::from_value(json!({
                    "model": {"fixture": {"limit": 3}}
                }))
                .unwrap(),
                ..Default::default()
            });
            view.page = PageId::Providers;
            view.prepare_editors(window, cx);
            run(view, window, cx);
        })
    });
}

#[::core::prelude::v1::test]
fn editor_growth_counts_unicode_characters() {
    assert_eq!(editor_max_rows("я\n"), 3);
}

#[::core::prelude::v1::test]
fn invalid_form_and_json_text_survive_ordinary_mode_switches() {
    with_view(|view, window, cx| {
        let path = FieldPath::new("model", "fixture", "limit");
        let TextInput::Line(input) = &view.editors.fields[&path].input else {
            panic!("integer field must be a line");
        };
        let input = input.clone();
        input.update(cx, |input, cx| input.set_value("unfinished", window, cx));
        view.text_changed(
            &path,
            TextFormat::Integer {
                minimum: Some(0),
                maximum: None,
            },
            None,
            "unfinished",
            cx,
        );
        view.switch_mode("model", "fixture", ParameterMode::Json, cx);
        assert!(!view.json.contains(&module_key("model", "fixture")));
        assert_eq!(input.read(cx).value().as_ref(), "unfinished");
        assert!(view.editors.form_error("model", "fixture"));
        assert_eq!(view.draft.value(&path), Some(&json!(3)));
        view.switch_mode(
            "model",
            "fixture",
            ParameterMode::DiscardInvalidFormAndOpenJson,
            cx,
        );
        assert!(!view.editors.fields.contains_key(&path));
        view.prepare_editors(window, cx);
        let key = module_key("model", "fixture");
        let json_input = view.editors.json[&key].input.clone();
        json_input.update(cx, |input, cx| input.set_value("{", window, cx));
        view.json_changed(&key, "{", cx);
        view.switch_mode("model", "fixture", ParameterMode::Form, cx);
        assert!(view.json.contains(&key));
        assert_eq!(json_input.read(cx).value().as_ref(), "{");
        assert!(view.json_error("model", "fixture"));
        view.json_changed(&key, r#"{"limit":9}"#, cx);
        view.switch_mode("model", "fixture", ParameterMode::Form, cx);
        view.prepare_editors(window, cx);
        assert!(!view.json.contains(&key));
        assert_eq!(view.draft.value(&path), Some(&json!(9)));
        let TextInput::Line(input) = &view.editors.fields[&path].input else {
            panic!("integer field must be a line");
        };
        assert_eq!(input.read(cx).value().as_ref(), "9");
    });
}

#[::core::prelude::v1::test]
fn raw_parameter_search_targets_the_rendered_json_editor() {
    with_view(|view, _, cx| {
        let form_hits = view.search("Limit", cx);
        assert_eq!(form_hits.len(), 1);
        assert_eq!(
            form_hits[0].target.spot,
            Some(super::super::spot::field(&FieldPath::new(
                "model", "fixture", "limit"
            )))
        );
        view.switch_mode("model", "fixture", ParameterMode::Json, cx);
        let raw_hits = view.search("Limit", cx);
        assert_eq!(raw_hits.len(), 1);
        assert_eq!(
            raw_hits[0].target.spot,
            Some(super::super::spot::params_json("model", "fixture"))
        );
        assert!(raw_hits[0].target.open.is_empty());
    });
}
