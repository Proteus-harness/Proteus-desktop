use super::*;
use serde_json::json;

fn addons(value: serde_json::Value) -> AppAddonsUpdate {
    serde_json::from_value(value).unwrap()
}

fn plugins(paths: &[&str]) -> AppAddonsUpdate {
    addons(
        json!({"addons":{"plugins":paths.iter().map(|path| json!({"path":path})).collect::<Vec<_>>()},"mcp_servers":[]}),
    )
}

#[test]
fn plugin_replacement_carries_newer_switch_and_accepted_data_identity() {
    let before = plugins(&["old", "neighbor"]);
    let mut current = before.clone();
    current.addons.plugins[0].enabled = false;
    current.addons.plugins[1].enabled = false;
    let mut accepted = plugins(&["neighbor", "new"]);
    accepted.addons.plugins[1].data_key = Some("stable-origin".into());
    let change = CatalogChange {
        before,
        replacement: Some(Replacement::Plugin {
            old: "old".into(),
            new: "new".into(),
        }),
    };
    assert!(
        change
            .rebase(&mut current, &accepted, &BTreeSet::new())
            .is_empty()
    );
    assert_eq!(current.addons.plugins.len(), 2);
    assert_eq!(current.addons.plugins[0].path, PathBuf::from("new"));
    assert_eq!(
        current.addons.plugins[0].data_key.as_deref(),
        Some("stable-origin")
    );
    assert!(!current.addons.plugins[0].enabled);
    assert!(!current.addons.plugins[1].enabled);
}

#[test]
fn intentional_removal_during_version_update_is_an_explicit_conflict_without_duplicate() {
    let change = CatalogChange {
        before: plugins(&["old"]),
        replacement: Some(Replacement::Plugin {
            old: "old".into(),
            new: "new".into(),
        }),
    };
    let mut current = plugins(&[]);
    let conflicts = change.rebase(&mut current, &plugins(&["new"]), &BTreeSet::new());
    assert_eq!(conflicts, ["пакет old"]);
    assert!(current.addons.plugins.is_empty());
}

#[test]
fn catalog_mcp_removal_merges_other_connections_and_switches() {
    let before = addons(
        json!({"addons":{"disabled_mcp_servers":["removed"]},"mcp_servers":[
        {"name":"removed","command":"remove-me"},{"name":"local","command":"before"}]}),
    );
    let mut accepted = before.clone();
    accepted.mcp_servers.remove(0);
    accepted.addons.disabled_mcp_servers.clear();
    let mut current = before.clone();
    current.mcp_servers[1].command = "newer".into();
    current.addons.disabled_skills.push("local-skill".into());
    let change = CatalogChange {
        before,
        replacement: None,
    };
    assert!(
        change
            .rebase(&mut current, &accepted, &BTreeSet::new())
            .is_empty()
    );
    assert_eq!(current.mcp_servers.len(), 1);
    assert_eq!(current.mcp_servers[0].command, "newer");
    assert_eq!(current.addons.disabled_skills, ["local-skill"]);
    assert!(current.addons.disabled_mcp_servers.is_empty());
}

#[test]
fn raw_mcp_editor_protects_removed_server_and_its_switch() {
    let before = addons(
        json!({"addons":{"disabled_mcp_servers":["db"]},"mcp_servers":[{"name":"db","command":"db"}]}),
    );
    let mut current = before.clone();
    let change = CatalogChange {
        before,
        replacement: None,
    };
    let conflicts = change.rebase(&mut current, &plugins(&[]), &BTreeSet::from(["db".into()]));
    assert_eq!(conflicts, ["MCP db"]);
    assert_eq!(current.mcp_servers.len(), 1);
    assert_eq!(current.addons.disabled_mcp_servers, ["db"]);
}

#[test]
fn renamed_catalog_mcp_does_not_add_a_second_binding_on_local_conflict() {
    let before = addons(
        json!({"addons":{"disabled_mcp_servers":["old"]},"mcp_servers":[{"name":"old","command":"before"}]}),
    );
    let mut current = before.clone();
    current.mcp_servers[0].command = "local".into();
    let accepted = addons(
        json!({"addons":{"disabled_mcp_servers":["new"]},"mcp_servers":[{"name":"new","command":"catalog"}]}),
    );
    let change = CatalogChange {
        before,
        replacement: Some(Replacement::Mcp {
            old: "old".into(),
            new: "new".into(),
        }),
    };
    assert_eq!(
        change.rebase(&mut current, &accepted, &BTreeSet::new()),
        ["MCP old"]
    );
    assert_eq!(current.mcp_servers.len(), 1);
    assert_eq!(current.mcp_servers[0].name, "old");
    assert_eq!(current.mcp_servers[0].command, "local");
    assert_eq!(current.addons.disabled_mcp_servers, ["old"]);
}

#[test]
fn renamed_catalog_mcp_carries_a_newer_disabled_switch() {
    for initially_disabled in [false, true] {
        let mut before =
            addons(json!({"addons":{},"mcp_servers":[{"name":"old","command":"before"}]}));
        let mut accepted =
            addons(json!({"addons":{},"mcp_servers":[{"name":"new","command":"catalog"}]}));
        if initially_disabled {
            before.addons.disabled_mcp_servers.push("old".into());
            accepted.addons.disabled_mcp_servers.push("new".into());
        }
        let mut current = before.clone();
        current.addons.disabled_mcp_servers = if initially_disabled {
            vec![]
        } else {
            vec!["old".into()]
        };
        let change = CatalogChange {
            before,
            replacement: Some(Replacement::Mcp {
                old: "old".into(),
                new: "new".into(),
            }),
        };
        assert!(
            change
                .rebase(&mut current, &accepted, &BTreeSet::new())
                .is_empty()
        );
        assert_eq!(current.mcp_servers.len(), 1);
        assert_eq!(current.mcp_servers[0].name, "new");
        assert_eq!(
            current
                .addons
                .disabled_mcp_servers
                .contains(&"new".to_string()),
            !initially_disabled
        );
        assert!(
            !current
                .addons
                .disabled_mcp_servers
                .contains(&"old".to_string())
        );
    }
}

#[test]
fn repeated_ack_is_idempotent_and_independent_plugin_additions_are_not_lost() {
    let change = CatalogChange {
        before: plugins(&[]),
        replacement: None,
    };
    let accepted = plugins(&["catalog"]);
    let mut current = plugins(&["local"]);
    for _ in 0..2 {
        assert!(
            change
                .rebase(&mut current, &accepted, &BTreeSet::new())
                .is_empty()
        );
    }
    assert_eq!(
        current
            .addons
            .plugins
            .iter()
            .map(|plugin| plugin.path.as_path())
            .collect::<Vec<_>>(),
        [
            std::path::Path::new("local"),
            std::path::Path::new("catalog")
        ]
    );
}
