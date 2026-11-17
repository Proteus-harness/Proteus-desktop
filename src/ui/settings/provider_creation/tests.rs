//! Delayed completion through the registration handler, without HTTP/runtime.
use super::super::editors::TextInput;
use super::super::{LoadState, SettingsContext};
use super::*;
use crate::settings::{FieldPath, models::ModelText};
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderModule, ConfigBuilderSnapshot,
};
use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
use serde_json::json;
use std::collections::BTreeMap;

#[::core::prelude::v1::test]
fn registration_ack_preserves_external_connections_and_newer_invalid_editor() {
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
            let initial = ConfigBuilderSnapshot {
                writable: true,
                model: Some("main".into()),
                model_writable: true,
                models_writable: true,
                models: BTreeMap::from([(
                    "main".into(),
                    ConfigBuilderModelPreset {
                        provider: "original".into(),
                        model: "old-id".into(),
                        ..Default::default()
                    },
                )]),
                permission_mode: "normal".into(),
                model_modules: vec![
                    ConfigBuilderModule {
                        id: "original".into(),
                        slot: "model".into(),
                        active: true,
                        config_schema: Some(ModuleConfigSchema {
                            fields: vec![
                                ConfigField::new("route", "Route", "", ConfigValueSchema::text()),
                                ConfigField::new(
                                    "local",
                                    "Local",
                                    "",
                                    ConfigValueSchema::integer(0),
                                ),
                            ],
                        }),
                        ..Default::default()
                    },
                    ConfigBuilderModule {
                        id: "raw".into(),
                        slot: "model".into(),
                        ..Default::default()
                    },
                ],
                module_config: BTreeMap::from([(
                    "model".into(),
                    BTreeMap::from([
                        ("original".into(), json!({"route": "old", "local": 1})),
                        ("raw".into(), json!({"value": "old"})),
                    ]),
                )]),
                ..Default::default()
            };
            view.accept(initial.clone());
            view.page = PageId::Models;
            view.open.insert(super::super::models::model_key("main"));
            view.prepare_editors(window, cx);
            let model_key = ("main".into(), ModelText::Model);
            let old_model_entity = view.editors.models.fields[&model_key].state.entity_id();
            view.page = PageId::Providers;
            view.prepare_editors(window, cx);
            let route = FieldPath::new("model", "original", "route");
            let TextInput::Line(old_route) = &view.editors.fields[&route].input else {
                panic!("route input")
            };
            let old_route_entity = old_route.entity_id();
            let raw = ("model".into(), "raw".into());
            let old_raw_entity = view.editors.json[&raw].input.entity_id();
            let previous = view.baseline.clone();
            let submitted_revision = view.edit_revision;
            view.catalogs.busy = true;

            view.update_draft(cx, |draft| {
                draft
                    .config
                    .get_mut("model")
                    .unwrap()
                    .get_mut("original")
                    .unwrap()["local"] = json!(2);
                draft.set_tool("late-tool", true);
            });
            view.ensure_json("tool", "invalid", window, cx);
            let key = ("tool".into(), "invalid".into());
            let input = view.editors.json[&key].input.clone();
            input.update(cx, |input, cx| input.set_value("{invalid", window, cx));
            view.json_changed(&key, "{invalid", cx);

            let mut accepted = initial;
            accepted.model = Some("external-choice".into());
            accepted.models.get_mut("main").unwrap().model = "updated-id".into();
            accepted.models.insert(
                "external-choice".into(),
                ConfigBuilderModelPreset {
                    provider: "other-window".into(),
                    model: "external-id".into(),
                    ..Default::default()
                },
            );
            accepted.module_config.get_mut("model").unwrap().extend([
                (
                    "created-here".into(),
                    json!({"provider": "go", "required": true}),
                ),
                (
                    "other-window".into(),
                    json!({"provider": "zen", "required": true}),
                ),
                ("original".into(), json!({"route": "updated", "local": 1})),
                ("raw".into(), json!({"value": "updated"})),
            ]);
            accepted
                .model_modules
                .extend(
                    ["created-here", "other-window"].map(|id| ConfigBuilderModule {
                        id: id.into(),
                        slot: "model".into(),
                        ..Default::default()
                    }),
                );
            view.finish_provider_registration(Ok(accepted), submitted_revision, &previous, cx);
            view.page = PageId::Models;
            view.prepare_editors(window, cx);
            let model = &view.editors.models.fields[&model_key].state;
            assert_ne!(model.entity_id(), old_model_entity);
            assert_eq!(model.read(cx).value().as_ref(), "updated-id");
            view.page = PageId::Providers;
            view.prepare_editors(window, cx);
            let TextInput::Line(route_input) = &view.editors.fields[&route].input else {
                panic!("route input")
            };
            assert_ne!(route_input.entity_id(), old_route_entity);
            assert_eq!(route_input.read(cx).value().as_ref(), "updated");
            let raw_input = &view.editors.json[&raw].input;
            assert_ne!(raw_input.entity_id(), old_raw_entity);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&raw_input.read(cx).value()).unwrap(),
                json!({"value": "updated"})
            );

            assert!(matches!(view.state, LoadState::Ready(_)));
            assert!(!view.catalogs.busy);
            assert_eq!(view.editors.json[&key].input.entity_id(), input.entity_id());
            assert_eq!(input.read(cx).value().as_ref(), "{invalid");
            assert!(view.editors.has_errors());
            assert!(!view.can_save());
            let next = view.draft.changes(&view.baseline);
            assert!(
                next.model.is_none(),
                "Untouched external root selection must stay saved"
            );
            assert!(
                next.models.is_none(),
                "Untouched updated model catalog must stay saved"
            );
            assert_eq!(
                next.module_config["model"]["original"],
                json!({"route": "updated", "local": 2})
            );
            for id in ["created-here", "other-window"] {
                assert_eq!(
                    next.module_config["model"][id],
                    view.baseline.config["model"][id]
                );
            }
            assert_eq!(next.tools_enabled, Some(vec!["late-tool".into()]));
            // Correct the invalid raw text: the subsequent real Save payload must
            // retain every accepted connection despite replacing the whole slot.
            input.update(cx, |input, cx| input.set_value("{}", window, cx));
            view.json_changed(&key, "{}", cx);
            assert!(view.can_save());
            let next = view.draft.changes(&view.baseline);
            assert_eq!(next.module_config["model"].len(), 4);
            assert!(next.model.is_none());
            assert!(next.models.is_none());
        })
    });
}

fn test_context() -> ::gpui::TestAppContext {
    let cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    cx
}

fn new_view(
    cx: &mut ::gpui::TestAppContext,
) -> (&mut ::gpui::VisualTestContext, Entity<SettingsView>) {
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
    (window, view)
}

#[::core::prelude::v1::test]
fn registration_refreshes_only_the_untouched_mode_text_and_retains_pending_id_and_tag_query() {
    use crate::settings::modes::{ModeList, ModeText};
    use proteus_contracts::domain::BehaviorModeDefinition;
    for local_title in [false, true] {
        let mut cx = test_context();
        let (window, view) = new_view(&mut cx);
        let (previous, submitted, id, query, title, instructions) = window.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.accept(ConfigBuilderSnapshot {
                    writable: true,
                    modes_writable: true,
                    permission_mode: "normal".into(),
                    modes: BTreeMap::from([(
                        "review".into(),
                        BehaviorModeDefinition {
                            title: Some("Old title".into()),
                            instructions: Some("Old instructions".into()),
                            ..Default::default()
                        },
                    )]),
                    ..Default::default()
                });
                view.page = PageId::Modes;
                view.open.insert(super::super::modes::mode_key("review"));
                view.prepare_editors(window, cx);
                let fields = &view.editors.modes.fields["review"];
                (
                    view.baseline.clone(),
                    view.edit_revision,
                    fields.id.clone(),
                    fields.queries[&ModeList::Tools].clone(),
                    fields.title.clone(),
                    fields.instructions.clone(),
                )
            })
        });
        // Real raw-input callbacks must advance the revision even when the
        // draft is still clean, or a clean ACK would discard these controls.
        window.update(|window, cx| {
            id.update(cx, |input, cx| {
                input.set_value("pending-rename", window, cx);
                cx.emit(InputEvent::Change);
            });
            query.update(cx, |input, cx| {
                input.set_value("unknown-tool", window, cx);
                cx.emit(InputEvent::Change);
            });
        });
        window.run_until_parked();
        window.update(|window, cx| {
            view.update(cx, |view, cx| {
                assert_ne!(view.edit_revision, submitted);
                assert!(!view.dirty());
                if local_title {
                    title.update(cx, |input, cx| input.set_value("Local title", window, cx));
                    view.behavior_text_changed("review", ModeText::Title, "Local title", cx);
                }
                let mut accepted = view.snapshot().unwrap().clone();
                accepted.modes.get_mut("review").unwrap().title = Some("Remote title".into());
                accepted.modes.get_mut("review").unwrap().instructions =
                    Some("Remote instructions".into());
                view.finish_provider_registration(Ok(accepted), submitted, &previous, cx);
                view.prepare_editors(window, cx);
                let fields = &view.editors.modes.fields["review"];
                assert_eq!(fields.id.entity_id(), id.entity_id());
                assert_eq!(fields.id.read(cx).value().as_ref(), "pending-rename");
                assert_eq!(
                    fields.queries[&ModeList::Tools].entity_id(),
                    query.entity_id()
                );
                assert_eq!(query.read(cx).value().as_ref(), "unknown-tool");
                assert_eq!(
                    fields.title.read(cx).value().as_ref(),
                    if local_title {
                        "Local title"
                    } else {
                        "Remote title"
                    }
                );
                assert_eq!(fields.title.entity_id() == title.entity_id(), local_title);
                assert_ne!(fields.instructions.entity_id(), instructions.entity_id());
                assert_eq!(
                    fields.instructions.read(cx).value().as_ref(),
                    "Remote instructions"
                );
                let next = view.draft.changes(&view.baseline);
                assert_eq!(next.modes.is_some(), local_title);
                if let Some(modes) = next.modes {
                    assert_eq!(
                        modes["review"].instructions.as_deref(),
                        Some("Remote instructions")
                    );
                }
            })
        });
    }
}

#[::core::prelude::v1::test]
fn registration_keeps_format_only_json_and_model_buffers_and_cancels_refresh_on_later_edits() {
    let mut cx = test_context();
    let (window, view) = new_view(&mut cx);
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            let mut model = ConfigBuilderModelPreset {
                provider: "original".into(),
                model: "old".into(),
                ..Default::default()
            };
            model.reasoning.budget_tokens = Some(8);
            view.accept(ConfigBuilderSnapshot {
                writable: true,
                models_writable: true,
                permission_mode: "normal".into(),
                models: BTreeMap::from([("main".into(), model)]),
                module_config: BTreeMap::from([(
                    "model".into(),
                    BTreeMap::from([("raw".into(), json!({"a": 1}))]),
                )]),
                ..Default::default()
            });
            view.open.insert(super::super::models::model_key("main"));
            view.prepare_model_editors(window, cx);
            view.ensure_json("model", "raw", window, cx);
            let previous = view.baseline.clone();
            let submitted = view.edit_revision;
            let budget_key = ("main".into(), ModelText::Budget);
            let budget = view.editors.models.fields[&budget_key].state.clone();
            budget.update(cx, |input, cx| input.set_value("0008", window, cx));
            view.model_text_changed("main", ModelText::Budget, "0008", cx);
            let key = ("model".into(), "raw".into());
            let json = view.editors.json[&key].input.clone();
            json.update(cx, |input, cx| input.set_value("{  \"a\": 1 }", window, cx));
            view.json_changed(&key, "{  \"a\": 1 }", cx);
            assert!(!view.dirty(), "These are only raw formatting edits");
            let model_key = ("main".into(), ModelText::Model);
            let model_input = view.editors.models.fields[&model_key].state.clone();
            let mut accepted = view.snapshot().unwrap().clone();
            accepted.models.get_mut("main").unwrap().model = "remote-id".into();
            accepted
                .models
                .get_mut("main")
                .unwrap()
                .reasoning
                .budget_tokens = Some(12);
            accepted
                .module_config
                .get_mut("model")
                .unwrap()
                .insert("raw".into(), json!({"a": 2}));
            view.finish_provider_registration(Ok(accepted), submitted, &previous, cx);
            // Input can arrive after the ACK has marked this control but before
            // next-frame prepare. That edit cancels the pending refresh.
            model_input.update(cx, |input, cx| input.set_value("local-id", window, cx));
            view.model_text_changed("main", ModelText::Model, "local-id", cx);
            view.prepare_model_editors(window, cx);
            view.ensure_json("model", "raw", window, cx);
            assert_eq!(
                view.editors.models.fields[&model_key].state.entity_id(),
                model_input.entity_id()
            );
            assert_eq!(model_input.read(cx).value().as_ref(), "local-id");
            assert_eq!(
                view.editors.models.fields[&budget_key].state.entity_id(),
                budget.entity_id()
            );
            assert_eq!(budget.read(cx).value().as_ref(), "0008");
            assert_eq!(view.editors.json[&key].input.entity_id(), json.entity_id());
            assert_eq!(json.read(cx).value().as_ref(), "{  \"a\": 1 }");
            let next = view.draft.changes(&view.baseline);
            assert!(
                next.module_config.is_empty(),
                "Retained raw text must not undo accepted JSON"
            );
            let models = next.models.unwrap();
            assert_eq!(models["main"].model, "local-id");
            assert_eq!(models["main"].reasoning.budget_tokens, Some(12));
        })
    });
}

#[::core::prelude::v1::test]
fn registration_keeps_a_late_schema_tag_query_without_a_parsed_draft_change() {
    let mut cx = test_context();
    let (window, view) = new_view(&mut cx);
    let path = FieldPath::new("model", "original", "sources");
    let (previous, submitted, query) = window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(ConfigBuilderSnapshot {
                writable: true,
                permission_mode: "normal".into(),
                model_modules: vec![ConfigBuilderModule {
                    id: "original".into(),
                    slot: "model".into(),
                    config_schema: Some(ModuleConfigSchema {
                        fields: vec![ConfigField::new(
                            "sources",
                            "Sources",
                            "",
                            ConfigValueSchema::strings(),
                        )],
                    }),
                    ..Default::default()
                }],
                module_config: BTreeMap::from([(
                    "model".into(),
                    BTreeMap::from([("original".into(), json!({"sources": ["old"]}))]),
                )]),
                ..Default::default()
            });
            view.page = PageId::Providers;
            view.prepare_editors(window, cx);
            let TextInput::Line(query) = &view.editors.fields[&path].input else {
                panic!("tag query")
            };
            (view.baseline.clone(), view.edit_revision, query.clone())
        })
    });
    window.update(|window, cx| {
        query.update(cx, |input, cx| {
            input.set_value("pending source", window, cx);
            cx.emit(InputEvent::Change);
        })
    });
    window.run_until_parked();
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            assert_ne!(view.edit_revision, submitted);
            assert!(!view.dirty());
            let mut accepted = view.snapshot().unwrap().clone();
            accepted
                .module_config
                .get_mut("model")
                .unwrap()
                .insert("original".into(), json!({"sources": ["remote"]}));
            view.finish_provider_registration(Ok(accepted), submitted, &previous, cx);
            view.prepare_editors(window, cx);
            let TextInput::Line(current) = &view.editors.fields[&path].input else {
                panic!("tag query")
            };
            assert_eq!(current.entity_id(), query.entity_id());
            assert_eq!(current.read(cx).value().as_ref(), "pending source");
            assert_eq!(view.draft.value(&path), Some(&json!(["remote"])));
            assert!(!view.dirty());
            assert!(view.draft.changes(&view.baseline).module_config.is_empty());
        })
    });
}

#[::core::prelude::v1::test]
fn removal_ack_keeps_unrelated_edits_and_cannot_restore_removed_connection_settings() {
    let mut cx = test_context();
    let (window, view) = new_view(&mut cx);
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(ConfigBuilderSnapshot {
                writable: true,
                permission_mode: "normal".into(),
                models: BTreeMap::from([(
                    "main".into(),
                    ConfigBuilderModelPreset {
                        provider: "retained".into(),
                        model: "model".into(),
                        ..Default::default()
                    },
                )]),
                model_modules: ["removed", "retained"]
                    .map(|id| ConfigBuilderModule {
                        id: id.into(),
                        slot: "model".into(),
                        ..Default::default()
                    })
                    .into(),
                module_config: BTreeMap::from([(
                    "model".into(),
                    BTreeMap::from([
                        ("removed".into(), json!({"url": "old"})),
                        ("retained".into(), json!({"url": "keep"})),
                    ]),
                )]),
                ..Default::default()
            });
            assert_eq!(view.provider_models("retained"), ["main"]);
            assert!(view.provider_models("removed").is_empty());
            let previous = view.baseline.clone();
            let submitted = view.edit_revision;
            view.catalogs.busy = true;
            view.ensure_json("model", "removed", window, cx);
            view.update_draft(cx, |draft| {
                draft.set_module_config(
                    "model",
                    "removed",
                    serde_json::from_value(json!({"url": "late"})).unwrap(),
                );
                draft.set_tool("late-tool", true);
            });
            let mut accepted = view.snapshot().unwrap().clone();
            accepted
                .model_modules
                .retain(|module| module.id != "removed");
            accepted
                .module_config
                .get_mut("model")
                .unwrap()
                .remove("removed");
            view.finish_provider_removal("removed", Ok(accepted), submitted, &previous, cx);
            assert!(!view.catalogs.busy);
            assert!(view.draft.module_config("model", "removed").is_none());
            assert!(
                !view
                    .editors
                    .json
                    .contains_key(&("model".into(), "removed".into()))
            );
            assert_eq!(
                view.draft.module_config("model", "retained"),
                Some(&json!({"url":"keep"}))
            );
            let request = view.draft.changes(&view.baseline);
            assert!(request.module_config.is_empty());
            assert_eq!(request.tools_enabled, Some(vec!["late-tool".into()]));
        })
    });
}
