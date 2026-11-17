//! Presets, source role references and history move together, independently
//! from provider configuration and transient per-chat model choices.
use super::super::{
    Draft,
    models::{ModelText, model_problems, set_model_text},
};
use crate::settings::{
    catalog::{PageId, area_page},
    draft::Area,
};
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderModelRole, ConfigBuilderSnapshot, ConfigBuilderState,
};
use std::collections::BTreeMap;

fn preset(model: &str) -> ConfigBuilderModelPreset {
    ConfigBuilderModelPreset {
        provider: "connection".into(),
        model: model.into(),
        stream: true,
        ..Default::default()
    }
}
fn snapshot() -> ConfigBuilderSnapshot {
    ConfigBuilderSnapshot {
        model: Some("main".into()),
        models_writable: true,
        model_writable: true,
        models: [
            ("main".into(), preset("same-model")),
            ("cheap".into(), preset("same-model")),
        ]
        .into_iter()
        .collect(),
        model_roles: vec![
            ConfigBuilderModelRole {
                name: "explore".into(),
                model: None,
                editable: true,
                description: "inherits".into(),
            },
            ConfigBuilderModelRole {
                name: "review".into(),
                model: Some("cheap".into()),
                editable: true,
                description: "explicit source alias".into(),
            },
            ConfigBuilderModelRole {
                name: "external".into(),
                model: None,
                editable: false,
                description: "own source".into(),
            },
        ],
        ..Default::default()
    }
}

#[test]
fn model_presets_and_role_choices_are_independent_minimal_draft_changes() {
    let snapshot = snapshot();
    let baseline = Draft::from_snapshot(&snapshot);
    assert_eq!(
        baseline.role_models,
        [
            ("explore".into(), None),
            ("review".into(), Some("cheap".into()))
        ]
        .into_iter()
        .collect()
    );
    let mut draft = baseline.clone();
    // Equal definitions do not turn inherited or main roles into cheap references.
    set_model_text(
        draft.models.get_mut("cheap").unwrap(),
        ModelText::Effort,
        "high",
    )
    .unwrap();
    let request = draft.changes(&baseline);
    assert!(request.models.is_some());
    assert_eq!(request.expected_models.as_ref(), Some(&baseline.models));
    assert!(request.model.is_none());
    assert!(request.role_models.is_none());
    assert!(request.module_config.is_empty());
    assert_eq!(draft.areas(&baseline), [Area::Models]);
    assert_eq!(area_page(&Area::Models), PageId::Models);
    set_model_text(
        draft.models.get_mut("cheap").unwrap(),
        ModelText::Effort,
        "",
    )
    .unwrap();
    assert!(!draft.is_dirty(&baseline));
    draft.role_models.insert("review".into(), None);
    assert_eq!(
        draft.changes(&baseline).role_models,
        Some([("review".into(), None)].into_iter().collect())
    );
    draft
        .role_models
        .insert("review".into(), Some("cheap".into()));
    assert!(!draft.is_dirty(&baseline));
}

#[test]
fn history_restores_preset_definitions_choices_and_explicit_root_clear() {
    let snapshot = snapshot();
    let baseline = Draft::from_snapshot(&snapshot);
    let state = ConfigBuilderState {
        model: None,
        models: [("cheap".into(), preset("previous-model"))]
            .into_iter()
            .collect(),
        role_models: [
            ("explore".into(), Some("cheap".into())),
            ("review".into(), None),
        ]
        .into_iter()
        .collect(),
        ..Default::default()
    };
    let restored = Draft::from_revision(&snapshot, &state);
    assert_eq!(restored.models, state.models);
    assert_eq!(restored.role_models, state.role_models);
    let request = restored.changes(&baseline);
    assert_eq!(request.model, Some(None));
    assert_eq!(request.models, Some(state.models));
    assert_eq!(request.role_models, Some(state.role_models));
    assert!(model_problems(&restored, &snapshot).is_empty());
}

#[test]
fn removing_referenced_models_and_invalid_budget_are_not_silently_accepted() {
    let snapshot = snapshot();
    let mut draft = Draft::from_snapshot(&snapshot);
    draft.models.remove("cheap");
    assert!(
        model_problems(&draft, &snapshot)
            .iter()
            .any(|problem| problem.contains("review") && problem.contains("cheap"))
    );
    draft.models.remove("main");
    assert!(
        model_problems(&draft, &snapshot)
            .iter()
            .any(|problem| problem.contains("Основной агент"))
    );
    let mut model = preset("id");
    model.reasoning.budget_tokens = Some(64);
    assert!(set_model_text(&mut model, ModelText::Budget, "invalid").is_err());
    assert_eq!(model.reasoning.budget_tokens, Some(64));
    assert!(set_model_text(&mut model, ModelText::Budget, "4294967296").is_err());
    set_model_text(&mut model, ModelText::Budget, "").unwrap();
    assert_eq!(model.reasoning.budget_tokens, None);
    let mut readonly = snapshot;
    readonly.models_writable = false;
    draft.models = BTreeMap::new();
    assert!(
        model_problems(&draft, &readonly)
            .iter()
            .any(|problem| problem.contains("другому файлу"))
    );
}

#[test]
fn history_does_not_restore_foreign_catalog_or_readonly_model_selection() {
    let mut snapshot = snapshot();
    snapshot.model_writable = false;
    snapshot.models_writable = false;
    let baseline = Draft::from_snapshot(&snapshot);
    let state = ConfigBuilderState {
        model: Some("outdated".into()),
        models: [("outdated".into(), preset("foreign-old-definition"))]
            .into_iter()
            .collect(),
        role_models: baseline.role_models.clone(),
        permission_mode: "plan".into(),
        ..Default::default()
    };
    let restored = Draft::from_revision(&snapshot, &state);
    assert_eq!(restored.model, snapshot.model);
    assert_eq!(restored.models, snapshot.models);
    let request = restored.changes(&baseline);
    assert!(request.model.is_none());
    assert!(request.models.is_none());
    assert_eq!(
        serde_json::to_value(request.permission_mode).unwrap(),
        serde_json::json!("plan")
    );
    assert!(model_problems(&restored, &snapshot).is_empty());
}

#[test]
fn history_keeps_connections_added_after_revision_but_restores_existing_config() {
    use serde_json::json;
    let mut snapshot = snapshot();
    snapshot.module_config.insert(
        "model".into(),
        BTreeMap::from([
            ("old".into(), json!({"changed":true})),
            (
                "new".into(),
                json!({"implementation":"openai","api_key":"synthetic"}),
            ),
        ]),
    );
    let state = ConfigBuilderState {
        model_connections: vec!["old".into()],
        models: snapshot.models.clone(),
        model: snapshot.model.clone(),
        ..Default::default()
    };
    let draft = Draft::from_revision(&snapshot, &state);
    assert_eq!(draft.config["model"]["old"], json!({}));
    assert_eq!(
        draft.config["model"]["new"],
        snapshot.module_config["model"]["new"]
    );
    // Historical source structure does not invent deleted roles or erase new ones.
    assert_eq!(draft.role_models["review"], Some("cheap".into()));
    assert_eq!(draft.role_models["explore"], None);
}
