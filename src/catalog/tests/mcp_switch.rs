//! Pure catalog binding transitions; no files, processes, UI or HTTP.

use super::*;
use crate::settings::reconcile::{CatalogChange, Replacement};
use proteus_contracts::domain::ConfiguredMcpServerConfig;
use std::collections::BTreeSet;

fn receipt(name: &str, enabled: bool, version: &str) -> Installed {
    let server = serde_json::from_value(serde_json::json!({
        "name": name,
        "command": format!("fixture-{version}"),
        "enabled": enabled,
        "args": ["", " spaced ", "first\nsecond"],
    }))
    .unwrap();
    Installed {
        source: Source {
            url: "https://catalog.invalid/fixture".into(),
            branch: None,
        },
        entry: "fixture".into(),
        version: version.into(),
        kind: Kind::McpServer,
        profile: Some("fixture.toml".into()),
        target: Target::Mcp { server },
    }
}

fn server(installed: &Installed) -> &ConfiguredMcpServerConfig {
    let Target::Mcp { server } = &installed.target else {
        unreachable!("MCP fixture")
    };
    server
}

#[test]
fn catalog_updates_and_removals_preserve_the_effective_switch_not_manifest_defaults() {
    let actions: &[&[bool]] = &[&[], &[false], &[true], &[true, false], &[false, true]];
    for old_default in [false, true] {
        for new_default in [false, true] {
            for new_name in ["old", "new"] {
                for actions in actions {
                    let old = receipt("old", old_default, "v1");
                    let updated = receipt(new_name, new_default, "v2");
                    let neighbor = receipt("neighbor", false, "neighbor");
                    let mut draft = Draft::default();
                    install::connect(&mut draft, &old, None).unwrap();
                    install::connect(&mut draft, &neighbor, None).unwrap();
                    for enabled in *actions {
                        draft.set_mcp("old", *enabled);
                    }
                    let expected_enabled = draft.mcp_enabled("old");

                    install::connect(&mut draft, &updated, Some(&old)).unwrap();

                    assert_eq!(draft.mcp_enabled(new_name), expected_enabled);
                    let current = draft.configured_mcp(new_name).unwrap();
                    let mut expected = server(&updated).clone();
                    expected.enabled = current.enabled;
                    assert_eq!(*current, expected);
                    assert_eq!(draft.configured_mcp("neighbor"), Some(server(&neighbor)));
                    if new_name != "old" {
                        assert!(draft.configured_mcp("old").is_none());
                        assert!(
                            !draft
                                .addons
                                .addons
                                .disabled_mcp_servers
                                .contains(&"old".into())
                        );
                    }

                    // The new receipt still holds its manifest default, so the
                    // same binding predicate must also permit later removal.
                    install::disconnect(&mut draft, &updated).unwrap();
                    assert!(draft.configured_mcp(new_name).is_none());
                    assert_eq!(draft.configured_mcp("neighbor"), Some(server(&neighbor)));
                }
            }
        }
    }
}

#[test]
fn fresh_or_stale_receipts_keep_manifest_defaults_and_existing_tombstones() {
    let old = receipt("old", true, "v1");
    for previous in [None, Some(&old)] {
        for new_name in ["old", "new"] {
            for new_default in [false, true] {
                for old_disabled in [false, true] {
                    for new_disabled in [false, true] {
                        let updated = receipt(new_name, new_default, "v2");
                        let mut draft = Draft::default();
                        if old_disabled {
                            draft.set_mcp("old", false);
                        }
                        if new_disabled {
                            draft.set_mcp(new_name, false);
                        }
                        let disabled_before = draft.addons.addons.disabled_mcp_servers.clone();
                        let old_tombstone = !draft.mcp_enabled("old");
                        let new_tombstone = !draft.mcp_enabled(new_name);

                        install::connect(&mut draft, &updated, previous).unwrap();

                        let expected =
                            new_default && !new_tombstone && !(previous.is_some() && old_tombstone);
                        assert_eq!(draft.mcp_enabled(new_name), expected);
                        assert_eq!(draft.configured_mcp(new_name).unwrap().enabled, new_default);
                        for name in disabled_before {
                            assert!(draft.addons.addons.disabled_mcp_servers.contains(&name));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn repeated_updates_use_the_same_receipt_relation_after_a_user_switch() {
    let old = receipt("fixture", false, "v1");
    let second = receipt("fixture", false, "v2");
    let third = receipt("fixture", false, "v3");
    let mut draft = Draft::default();
    install::connect(&mut draft, &old, None).unwrap();
    draft.set_mcp("fixture", true);
    install::connect(&mut draft, &second, Some(&old)).unwrap();
    install::connect(&mut draft, &third, Some(&second)).unwrap();
    assert!(draft.mcp_enabled("fixture"));
    assert_eq!(
        draft.configured_mcp("fixture").unwrap().command,
        "fixture-v3"
    );
}

#[test]
fn non_switch_edits_and_name_collisions_still_leave_the_draft_unchanged() {
    let old = receipt("old", false, "v1");
    let updated = receipt("new", true, "v2");
    for field in [
        "command",
        "args",
        "env",
        "cwd",
        "timeout",
        "protocol",
        "limit",
        "collision",
    ] {
        let mut draft = Draft::default();
        install::connect(&mut draft, &old, None).unwrap();
        draft.set_mcp("old", true);
        if field == "collision" {
            install::connect(&mut draft, &receipt("new", true, "independent"), None).unwrap();
        } else {
            let current = &mut draft.addons.mcp_servers[0];
            match field {
                "command" => current.command.push_str("-edited"),
                "args" => current.args.push("edited".into()),
                "env" => {
                    current
                        .environment
                        .env
                        .insert("EDITED".into(), "value".into());
                }
                "cwd" => current.cwd = Some("edited-dir".into()),
                "timeout" => current.timeout_ms = Some(17),
                "protocol" => current.protocol_version.push_str("-edited"),
                "limit" => current.max_response_bytes = Some(17),
                _ => unreachable!(),
            }
        }
        let before = draft.clone();
        assert!(
            install::connect(&mut draft, &updated, Some(&old)).is_err(),
            "{field}"
        );
        assert_eq!(draft, before, "{field}");
        if field != "collision" {
            assert!(install::disconnect(&mut draft, &old).is_err(), "{field}");
            assert_eq!(draft, before, "{field}");
        }
    }
}

#[test]
fn accepted_catalog_delta_keeps_the_switch_and_unrelated_later_edits() {
    for new_name in ["old", "new"] {
        let old = receipt("old", false, "v1");
        let updated = receipt(new_name, false, "v2");
        let mut before = Draft::default();
        install::connect(&mut before, &old, None).unwrap();
        before.set_mcp("old", true);
        let mut accepted = before.clone();
        install::connect(&mut accepted, &updated, Some(&old)).unwrap();
        let mut current = before.clone();
        current.set_skill("later-local-edit", false);
        let change = CatalogChange {
            before: before.addons,
            replacement: Some(Replacement::Mcp {
                old: "old".into(),
                new: new_name.into(),
            }),
        };

        assert!(
            change
                .rebase(&mut current.addons, &accepted.addons, &BTreeSet::new())
                .is_empty()
        );
        assert!(current.mcp_enabled(new_name));
        assert_eq!(
            current.configured_mcp(new_name),
            accepted.configured_mcp(new_name)
        );
        assert!(!current.skill_enabled("later-local-edit"));
    }
}

#[test]
fn a_newer_overlapping_switch_remains_an_explicit_rebase_conflict() {
    for new_name in ["old", "new"] {
        let old = receipt("old", false, "v1");
        let updated = receipt(new_name, false, "v2");
        let mut before = Draft::default();
        install::connect(&mut before, &old, None).unwrap();
        let mut accepted = before.clone();
        install::connect(&mut accepted, &updated, Some(&old)).unwrap();
        let mut current = before.clone();
        current.set_mcp("old", true);
        let local = current.clone();
        let change = CatalogChange {
            before: before.addons,
            replacement: Some(Replacement::Mcp {
                old: "old".into(),
                new: new_name.into(),
            }),
        };

        assert_eq!(
            change.rebase(&mut current.addons, &accepted.addons, &BTreeSet::new()),
            ["MCP old"]
        );
        assert_eq!(current, local);
        assert!(current.mcp_enabled("old"));
    }
}
