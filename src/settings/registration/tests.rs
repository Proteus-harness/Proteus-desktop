use super::*;
use serde_json::json;

#[test]
fn registration_rebases_untouched_fields_and_json_siblings_before_the_next_save() {
    let mut old = Draft::default();
    old.model = Some("main".into());
    old.mode = "normal".into();
    old.modules.insert("context".into(), "old".into());
    old.models.insert(
        "main".into(),
        ConfigBuilderModelPreset {
            provider: "original".into(),
            model: "old-model".into(),
            ..Default::default()
        },
    );
    old.modes.insert(
        "review".into(),
        BehaviorModeDefinition {
            title: Some("Old title".into()),
            instructions: Some("Old instructions".into()),
            ..Default::default()
        },
    );
    old.config.insert(
        "model".into(),
        BTreeMap::from([(
            "original".into(),
            json!({
                "nested": {"local": 1, "external": 1, "removed": 1}, "untouched": true
            }),
        )]),
    );
    let mut local = old.clone();
    local.models.get_mut("main").unwrap().reasoning.effort = Some("high".into());
    local.modes.get_mut("review").unwrap().instructions = Some("Local instructions".into());
    local
        .config
        .get_mut("model")
        .unwrap()
        .get_mut("original")
        .unwrap()["nested"]["local"] = json!(2);
    local.tools.insert("local-tool".into());

    let mut saved = old.clone();
    saved.model = Some("other-window".into());
    saved.mode = "auto".into();
    saved.modules.insert("context".into(), "new".into());
    saved.models.get_mut("main").unwrap().model = "updated-model".into();
    saved.modes.get_mut("review").unwrap().title = Some("Updated title".into());
    let connections = saved.config.get_mut("model").unwrap();
    connections.insert(
        "created-here".into(),
        json!({"provider": "go", "required": true}),
    );
    connections.insert("created-elsewhere".into(), json!({"provider": "zen"}));
    connections.insert(
        "original".into(),
        json!({"nested": {"local": 1, "external": 2}, "untouched": false}),
    );
    saved.tools.insert("external-tool".into());

    local.rebase_registration(&old, &saved);
    assert_eq!(local.model, saved.model);
    assert_eq!(local.mode, saved.mode);
    assert_eq!(local.modules, saved.modules);
    assert_eq!(local.models["main"].model, "updated-model");
    assert_eq!(
        local.models["main"].reasoning.effort.as_deref(),
        Some("high")
    );
    assert_eq!(
        local.modes["review"].title.as_deref(),
        Some("Updated title")
    );
    assert_eq!(
        local.modes["review"].instructions.as_deref(),
        Some("Local instructions")
    );
    let request = local.changes(&saved);
    assert!(request.model.is_none());
    assert!(request.permission_mode.is_none());
    assert!(request.modules.is_empty());
    assert_eq!(
        request.module_config["model"]["original"],
        json!({
            "nested": {"local": 2, "external": 2}, "untouched": false
        })
    );
    for id in ["created-here", "created-elsewhere"] {
        assert_eq!(
            request.module_config["model"][id],
            saved.config["model"][id]
        );
    }
    assert_eq!(
        request.tools_enabled,
        Some(vec!["external-tool".into(), "local-tool".into()])
    );
    assert_eq!(
        request.models.as_ref().unwrap()["main"].model,
        "updated-model"
    );
    assert_eq!(request.expected_models.as_ref(), Some(&saved.models));
    assert_eq!(
        request.modes.as_ref().unwrap()["review"].title,
        saved.modes["review"].title
    );
    assert_eq!(request.expected_modes.as_ref(), Some(&saved.modes));
}

#[test]
fn absent_base_maps_keep_accepted_seeds_and_actual_local_additions() {
    let old = Draft::default();
    let mut local = old.clone();
    local.config.insert(
        "model".into(),
        BTreeMap::from([
            ("new".into(), json!({"nested": {"local": true}})),
            ("local-only".into(), json!({"keep": true})),
        ]),
    );
    let mut saved = old.clone();
    saved.config.insert(
        "model".into(),
        BTreeMap::from([
            (
                "new".into(),
                json!({"provider": "required", "nested": {"seed": true}}),
            ),
            ("external".into(), json!({"provider": "external"})),
        ]),
    );
    local.rebase_registration(&old, &saved);
    let next = local.changes(&saved);
    assert_eq!(
        next.module_config["model"]["new"],
        json!({
            "provider": "required", "nested": {"seed": true, "local": true}
        })
    );
    assert_eq!(
        next.module_config["model"]["external"],
        saved.config["model"]["external"]
    );
    assert_eq!(
        next.module_config["model"]["local-only"],
        json!({"keep": true})
    );
}

#[test]
fn local_deletions_and_scalar_conflicts_survive_registration() {
    let mut old = Draft::default();
    old.config.insert(
        "tool".into(),
        BTreeMap::from([(
            "files".into(),
            json!({
                "removed-locally": 1, "changed-locally": 1, "external": 1
            }),
        )]),
    );
    let mut local = old.clone();
    local.config.get_mut("tool").unwrap().insert(
        "files".into(),
        json!({"changed-locally": null, "external": 1}),
    );
    let mut saved = old.clone();
    saved.config.get_mut("tool").unwrap().insert(
        "files".into(),
        json!({"removed-locally": 2, "changed-locally": 2, "external": 2}),
    );
    local.rebase_registration(&old, &saved);
    assert_eq!(
        local.config["tool"]["files"],
        json!({"changed-locally": null, "external": 2})
    );
}
