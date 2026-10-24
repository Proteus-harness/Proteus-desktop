use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
use serde_json::json;

use crate::settings::catalog::{editable_field, editable_params};

fn object(fields: Vec<ConfigField>) -> ConfigField {
    ConfigField::new("group", "Group", "", ConfigValueSchema::Object { fields })
}

#[test]
fn absent_or_empty_schema_and_config_have_no_parameter_editor() {
    let empty = ModuleConfigSchema { fields: Vec::new() };
    for schema in [None, Some(&empty)] {
        assert!(!editable_params(schema, None));
        assert!(!editable_params(schema, Some(&json!({}))));
    }
}

#[test]
fn scalar_and_nested_leaf_fields_are_editable_without_saved_values() {
    let leaf = ConfigField::new("enabled", "Enabled", "", ConfigValueSchema::Boolean {});
    let scalar = ModuleConfigSchema {
        fields: vec![leaf.clone()],
    };
    assert!(editable_params(Some(&scalar), None));
    let nested = ModuleConfigSchema {
        fields: vec![object(vec![object(vec![leaf])])],
    };
    assert!(editable_params(Some(&nested), Some(&json!({}))));
}

#[test]
fn nested_empty_groups_do_not_create_forms_or_advanced_disclosures() {
    let empty = object(Vec::new());
    assert!(!editable_field(&empty));
    let mut advanced = object(vec![object(vec![empty])]);
    advanced.advanced = true;
    let schema = ModuleConfigSchema {
        fields: vec![advanced],
    };
    assert!(!editable_params(Some(&schema), Some(&json!({}))));
}

#[test]
fn saved_unknown_keys_keep_json_access_with_missing_or_empty_schema() {
    let empty = ModuleConfigSchema { fields: Vec::new() };
    let groups = ModuleConfigSchema {
        fields: vec![object(vec![object(Vec::new())])],
    };
    for schema in [None, Some(&empty), Some(&groups)] {
        assert!(editable_params(schema, Some(&json!({"legacy": false}))));
    }
}

#[test]
fn selectors_without_choices_are_not_actionable_but_json_fields_are() {
    let empty = ConfigField::new(
        "mode",
        "Mode",
        "",
        ConfigValueSchema::Enum {
            options: Vec::new(),
        },
    );
    assert!(!editable_field(&empty));
    assert!(editable_field(&ConfigField::new(
        "extra",
        "Extra",
        "",
        ConfigValueSchema::Json {}
    )));
}
