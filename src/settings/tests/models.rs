//! Presets and source role references are independent from provider
//! configuration and transient per-chat model choices.
use super::super::{
    Draft,
    models::{ModelText, model_problems, set_model_text},
};
use crate::settings::{
    catalog::{PageId, area_page},
    draft::Area,
};
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderModelRole, ConfigBuilderSnapshot,
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
        writable: true,
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
    draft.model = None;
    assert_eq!(draft.changes(&baseline).model, Some(None));
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
fn editing_a_role_keeps_the_main_model_when_both_share_a_definition() {
    use super::super::models::ModelTarget;
    let mut snapshot = snapshot();
    snapshot.model_roles[1].model = snapshot.model.clone();
    snapshot
        .models
        .insert("model-role-review".into(), preset("unrelated"));
    let baseline = Draft::from_snapshot(&snapshot);
    let mut draft = baseline.clone();
    let review = ModelTarget::Role("review".into());
    let original = review.configured(&baseline).unwrap().clone();
    let mut choice = original.clone();
    choice.model = "review-only".into();
    choice.reasoning.summary = true;
    review.set_model(&mut draft, &baseline, &snapshot, Some(choice.clone()));
    assert_eq!(ModelTarget::Root.configured(&draft), Some(&original));
    assert_eq!(review.configured(&draft), Some(&choice));
    assert_eq!(
        draft.models["model-role-review"],
        baseline.models["model-role-review"]
    );
    assert!(draft.changes(&baseline).model.is_none());
    assert!(draft.changes(&baseline).role_models.is_some());

    review.set_model(&mut draft, &baseline, &snapshot, Some(original));
    assert_eq!(draft, baseline);
}

#[test]
fn inheriting_a_model_discards_only_an_uncommitted_role_definition() {
    use super::super::models::ModelTarget;
    let snapshot = snapshot();
    let baseline = Draft::from_snapshot(&snapshot);
    let mut draft = baseline.clone();
    let role = ModelTarget::Role("explore".into());
    role.set_model(&mut draft, &baseline, &snapshot, Some(preset("new-model")));
    assert_eq!(role.configured(&draft).unwrap().model, "new-model");
    role.set_model(&mut draft, &baseline, &snapshot, None);
    assert_eq!(draft, baseline);

    let existing_role = ModelTarget::Role("review".into());
    existing_role.set_model(&mut draft, &baseline, &snapshot, None);
    assert!(existing_role.configured(&draft).is_none());
    assert_eq!(draft.models, baseline.models);
}
