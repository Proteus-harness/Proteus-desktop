use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;
use crate::preferences::{Notifications, SendMode};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("build/verification/visual-config")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn agent(&self) -> PathBuf {
        let path = self.path("agent.toml");
        fs::write(&path, "model = 'retained-model'\n").unwrap();
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn pure_visual_json_and_toml_roundtrip_with_package_settings() {
    let fixture = Fixture::new();
    let mut value = VisualConfig::default();
    value.theme = ThemePreference::Light;
    value.fonts.chat = Some("Portable agent font".into());
    value.fonts.user_chat = Some("Portable user font".into());
    value.extensions.set_enabled("quota", false);
    value.package_visual_settings.insert(
        "plan".into(),
        serde_json::from_value(json!({"visibility":"always"})).unwrap(),
    );
    for name in ["visual.json", "visual.toml"] {
        let path = fixture.path(name);
        assert_eq!(save(&path, &value, None).unwrap(), path);
        assert_eq!(load(&path, None).unwrap(), value);
    }
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
}

#[test]
fn agent_and_device_parameters_are_not_a_visual_document() {
    let fixture = Fixture::new();
    let path = fixture.path("visual.json");
    for field in [
        "model",
        "module_config",
        "preferred_agent",
        "catalogs",
        "shortcuts",
        "tabs",
    ] {
        let document = serde_json::Map::from_iter([(field.to_owned(), json!({}))]);
        fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
        assert!(load(&path, None).is_err(), "accepted {field}");
    }
    fs::write(&path, r#"{"view":{"scale":123}}"#).unwrap();
    assert!(load(&path, None).is_err());
    fs::write(&path, "not a visual config").unwrap();
    assert!(load(&path, None).is_err());
}

#[test]
fn applying_visual_defaults_does_not_change_device_or_agent_metadata() {
    let mut preferences = Preferences::default();
    preferences.preferred_agent = crate::conversations::AgentKind::Codex;
    preferences.send = SendMode::CtrlEnter;
    preferences.notifications = Notifications::Off;
    preferences.visual_configs.default_config = Some(PathBuf::from("/visual/default.json"));
    let mut expected = serde_json::to_value(&preferences).unwrap();
    let mut visual = VisualConfig::from_preferences(&preferences);
    visual.theme = ThemePreference::Light;
    visual.view.interface_text = 20;
    visual.view.chat_width = 900;
    visual.view.settings_width = 1000;
    visual.extensions.set_enabled("quota", false);
    visual.apply_to_preferences(&mut preferences);
    // Only these independently selected presentation values changed.
    expected["theme"] = json!("light");
    expected["view"]["interface_text"] = json!(20);
    expected["view"]["chat_width"] = json!(900);
    expected["view"]["settings_width"] = json!(1000);
    expected["extensions"]["disabled"] = json!(["analysis", "quota"]);
    assert_eq!(serde_json::to_value(preferences).unwrap(), expected);
}

#[test]
fn bindings_use_canonical_paths_and_do_not_write_either_document() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let visual = fixture.path("visual.json");
    save(&visual, &VisualConfig::default(), Some(&agent)).unwrap();
    let agent_before = fs::read(&agent).unwrap();
    let visual_before = fs::read(&visual).unwrap();
    symlink(&agent, fixture.path("agent-alias.toml")).unwrap();
    symlink(&visual, fixture.path("visual-alias.json")).unwrap();
    let mut registry = Registry::default();
    registry
        .bind(
            &fixture.path("agent-alias.toml"),
            &fixture.path("visual-alias.json"),
        )
        .unwrap();
    let selected = registry.resolve(Some(&agent)).unwrap();
    assert_eq!(selected.path, Some(visual));
    assert_eq!(selected.origin, SelectionOrigin::Binding);
    assert_eq!(registry.bindings.len(), 1);
    assert_eq!(fs::read(agent).unwrap(), agent_before);
    assert_eq!(fs::read(selected.path.unwrap()).unwrap(), visual_before);
}

#[test]
fn manual_selection_survives_late_binding_and_two_windows_are_independent() {
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let bound = fixture.path("bound.json");
    let manual = fixture.path("manual.toml");
    save(&bound, &VisualConfig::default(), Some(&agent)).unwrap();
    save(&manual, &VisualConfig::default(), Some(&agent)).unwrap();
    let mut registry = Registry::default();
    registry.bind(&agent, &bound).unwrap();
    let mut first = registry.resolve(Some(&agent)).unwrap();
    let second = first.clone();
    first.select_manual(&manual, Some(&agent)).unwrap();
    assert!(!first.apply_agent_binding(&registry, Some(&agent)).unwrap());
    assert_eq!(first.path, Some(manual));
    assert_eq!(first.origin, SelectionOrigin::Manual);
    assert_eq!(second.path, Some(bound.clone()));
    assert_eq!(second.origin, SelectionOrigin::Binding);
    first.reset_to_auto(&registry, Some(&agent)).unwrap();
    assert_eq!(first.path, Some(bound));
}

#[test]
fn selecting_or_saving_an_agent_file_or_alias_is_refused_before_write() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let before = fs::read(&agent).unwrap();
    let symlink_path = fixture.path("visual.toml");
    let hardlink_path = fixture.path("hardlink.toml");
    symlink(&agent, &symlink_path).unwrap();
    fs::hard_link(&agent, &hardlink_path).unwrap();
    for path in [&agent, &symlink_path, &hardlink_path] {
        assert!(select_path(path, Some(&agent)).is_err());
        assert!(save(path, &VisualConfig::default(), Some(&agent)).is_err());
        assert_eq!(fs::read(&agent).unwrap(), before);
    }
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 3);
}

#[test]
fn failed_load_and_validation_preserve_existing_visual_document() {
    let fixture = Fixture::new();
    let path = fixture.path("visual.json");
    save(&path, &VisualConfig::default(), None).unwrap();
    let before = fs::read(&path).unwrap();
    let mut invalid = VisualConfig::default();
    invalid.view.chat_text = 1;
    assert!(save(&path, &invalid, None).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
}

#[test]
fn missing_binding_or_default_is_reported_instead_of_silently_using_builtin() {
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let mut registry = Registry::default();
    registry
        .bindings
        .insert(agent.clone(), fixture.path("missing.json"));
    assert!(registry.resolve(Some(&agent)).is_err());
    registry.bindings.clear();
    registry.default_config = Some(fixture.path("missing.toml"));
    assert!(registry.resolve(Some(&agent)).is_err());
    let mut manual = WindowSelection {
        path: None,
        origin: SelectionOrigin::Manual,
    };
    // An unrelated late update must not erase an explicit built-in selection.
    assert!(!manual.apply_agent_binding(&registry, Some(&agent)).unwrap());
}

#[test]
fn save_does_not_overwrite_another_agent_document_when_no_agent_is_selected() {
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let before = fs::read(&agent).unwrap();
    assert!(save(&agent, &VisualConfig::default(), None).is_err());
    assert_eq!(fs::read(&agent).unwrap(), before);
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 1);
}

#[test]
fn invalid_bind_and_default_choices_leave_existing_registry_unchanged() {
    let fixture = Fixture::new();
    let agent = fixture.agent();
    let visual = fixture.path("visual.json");
    save(&visual, &VisualConfig::default(), Some(&agent)).unwrap();
    let invalid = fixture.path("invalid.toml");
    fs::write(&invalid, "model = 'another-agent'\n").unwrap();
    let mut registry = Registry::default();
    registry.bind(&agent, &visual).unwrap();
    registry.set_default(Some(&visual)).unwrap();
    let before = registry.clone();
    assert!(registry.bind(&agent, &invalid).is_err());
    assert!(registry.set_default(Some(&invalid)).is_err());
    assert_eq!(registry, before);
}

#[test]
fn package_preference_limit_counts_encoded_utf8_and_escaping() {
    let mut value = VisualConfig::default();
    value.package_visual_settings.insert(
        "plan".into(),
        serde_json::Map::from_iter([("visibility".into(), json!("é".repeat(100)))]),
    );
    value.validate().unwrap();
    value
        .package_visual_settings
        .get_mut("plan")
        .unwrap()
        .insert("visibility".into(), json!("\n".repeat(33_000)));
    assert!(value.validate().is_err());
}
