use super::*;
use proteus_contracts::app_protocol::config_builder::ConfigBuilderModule;
use serde_json::json;

fn choice() -> Choice {
    Choice {
        package: "fixture".into(),
        command: vec!["/synthetic/module".into()],
        icon: Some("lucide:plug".into()),
        named: true,
        template: Template {
            id: "api".into(),
            title: "Synthetic API".into(),
            description: String::new(),
            module_id: "work".into(),
            module_config: json!({"implementation":"synthetic"}),
            env_allowlist: vec!["PATH".into(), "SYNTHETIC_KEY".into()],
        },
    }
}

#[test]
fn creation_instantiates_concrete_export_and_preserves_existing_connections_and_selection() {
    let mut snapshot = ConfigBuilderSnapshot::default();
    snapshot.model = Some("cheap".into());
    snapshot.module_config.insert(
        "model".into(),
        BTreeMap::from([(
            "existing".into(),
            json!({"provider":"unchanged","api_key":"synthetic-existing"}),
        )]),
    );
    let request = choice().request("personal", &snapshot).unwrap();
    assert_eq!(request.component_packages.len(), 1);
    let package = &request.component_packages[0];
    assert_eq!(package.exports["model"], vec!["personal"]);
    assert_eq!(package.icon.as_deref(), Some("lucide:plug"));
    assert_eq!(package.env_allowlist, vec!["PATH", "SYNTHETIC_KEY"]);
    assert!(
        request.module_config.is_empty(),
        "never send a stale whole-slot replacement"
    );
    assert_eq!(
        package.initial_config["model"]["personal"],
        json!({"implementation":"synthetic"})
    );
    assert!(!package.initial_config["model"].contains_key("existing"));
    assert!(request.model.is_none());
    assert!(request.models.is_none());
    assert!(request.role_models.is_none());
}

#[test]
fn instance_names_avoid_registered_and_unbound_config_keys_and_never_submit_wildcards() {
    let mut snapshot = ConfigBuilderSnapshot::default();
    snapshot.model_modules.push(ConfigBuilderModule {
        id: "work".into(),
        ..Default::default()
    });
    snapshot.module_config.insert(
        "model".into(),
        BTreeMap::from([("work-2".into(), json!({}))]),
    );
    let choice = choice();
    assert_eq!(choice.available_id(&snapshot), "work-3");
    for name in ["work", "work-2", "*", "", " bad", "x\ny"] {
        assert!(choice.request(name, &snapshot).is_err(), "{name:?}");
    }
    let mut fixed = choice.clone();
    fixed.named = false;
    assert!(fixed.request("other", &snapshot).is_err());
}

#[test]
fn module_metadata_and_executable_are_validated_before_offering_creation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build/verification")
        .join(format!("connection-template-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::write(root.join("bin/demo"), b"synthetic executable").unwrap();
    let path = root.join("module.json");
    let mut manifest = json!({"id":"fixture","version":"1","icon":"./logo.svg","command":["bin/demo"],"exports":{"model":["*"]},"sha256":format!("{:x}",Sha256::digest(b"synthetic executable")),"model_connections":[{"id":"synthetic","title":"Synthetic","description":"test","module_id":"work","module_config":{"implementation":"synthetic"},"env_allowlist":["PATH"]}]});
    std::fs::write(&path, manifest.to_string()).unwrap();
    let choices = read_choices(&path, &root).unwrap();
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].icon.as_deref(), root.join("./logo.svg").to_str());
    assert!(choices[0].named);
    assert!(Path::new(&choices[0].command[0]).is_absolute());
    std::fs::write(root.join("bin/demo"), b"modified executable").unwrap();
    assert!(read_choices(&path, &root).is_err());
    manifest["model_connections"][0]["module_id"] = json!("*");
    std::fs::write(&path, manifest.to_string()).unwrap();
    assert!(read_choices(&path, &root).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
