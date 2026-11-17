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

#[test]
fn discovery_skips_sdk_directories_and_nonproviders_but_reports_broken_provider_packages() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build/verification")
        .join(format!("connection-discovery-{}", uuid::Uuid::new_v4()));
    let modules = root.join("share/modules");
    for id in ["claude-command-hooks", "hook-process", "nonprovider"] {
        std::fs::create_dir_all(modules.join(id)).unwrap();
    }
    std::fs::write(
        modules.join("nonprovider/module.json"),
        json!({
            "id": "nonprovider", "version": "1", "command": ["bin/not-installed"],
            "exports": {"hook": ["synthetic"]}, "sha256": "not-validated-for-nonproviders"
        })
        .to_string(),
    )
    .unwrap();
    let discover = || {
        discover_with_inventory(
            &[],
            Inventory::read(
                Some(&root.join("bin/proteus")),
                &root.join("extensions"),
                None,
            ),
        )
    };
    let discovery = discover();
    assert!(discovery.choices.is_empty());
    assert!(discovery.problems.is_empty(), "{:?}", discovery.problems);
    std::fs::write(root.join("not-directory"), b"synthetic file").unwrap();
    assert!(read_choices(&root.join("not-directory/module.json"), &root).is_err());

    for id in ["malformed-provider", "missing-executable"] {
        std::fs::create_dir_all(modules.join(id)).unwrap();
    }
    std::fs::write(modules.join("malformed-provider/module.json"), b"{").unwrap();
    std::fs::write(
        modules.join("missing-executable/module.json"),
        json!({
            "id": "missing-executable", "version": "1", "command": ["bin/not-installed"],
            "exports": {"model": ["*"]}, "sha256": "unavailable",
            "model_connections": [{
                "id": "synthetic", "title": "Synthetic", "description": "test",
                "module_id": "work", "module_config": {}, "env_allowlist": []
            }]
        })
        .to_string(),
    )
    .unwrap();
    let discovery = discover();
    assert!(discovery.choices.is_empty());
    assert_eq!(discovery.problems.len(), 2, "{:?}", discovery.problems);
    for id in ["malformed-provider", "missing-executable"] {
        assert!(
            discovery
                .problems
                .iter()
                .any(|problem| problem.starts_with(&format!("{id}:"))),
            "{id}: {:?}",
            discovery.problems
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
