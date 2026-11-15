//! Exact MCP collection editing, independent of native UI and process launch.

use std::collections::BTreeMap;

use crate::settings::Draft;
use crate::settings::mcp::{McpText, mcp_text};

fn baseline() -> Draft {
    let mut draft = Draft::default();
    let ix = draft.add_mcp();
    draft.edit_mcp(ix, |server| {
        server.name = "fixture".into();
        server.command = "fixture-mcp".into();
        server.args = [
            "--empty",
            "",
            "  alpha  ",
            "first\nsecond",
            "cr\r\ntab\t",
            "quote\"slash\\",
            "Привет 😀",
            "same",
            "same",
        ]
        .map(str::to_owned)
        .to_vec();
        server.environment.env = BTreeMap::from([
            ("EMPTY".into(), "".into()),
            ("PREFIX".into(), "  synthetic-prefix  ".into()),
            ("MULTILINE".into(), "first\nsecond\r\n".into()),
            ("ESCAPES".into(), "quote\"slash\\tab\t".into()),
            ("UNICODE".into(), "Привет 😀".into()),
        ]);
        server.environment.env_allowlist = vec!["PATH".into()];
    });
    draft
}

#[test]
fn existing_mcp_values_round_trip_without_a_dirty_draft() {
    let baseline = baseline();
    let mut draft = baseline.clone();
    for field in [McpText::Args, McpText::Env] {
        let text = mcp_text(&draft.addons.mcp_servers[0], field);
        draft.set_mcp_text(0, field, &text).unwrap();
    }
    assert_eq!(draft, baseline);
    assert!(!draft.is_dirty(&baseline));
}

#[test]
fn editing_neighbor_arguments_preserves_prior_values_and_can_be_undone() {
    let baseline = baseline();
    let mut draft = baseline.clone();
    let original = &baseline.addons.mcp_servers[0].args;
    let mut args = original.clone();
    args.extend(["--tag".into(), "new".into()]);
    draft
        .set_mcp_text(0, McpText::Args, &serde_json::to_string(&args).unwrap())
        .unwrap();
    assert_eq!(
        &draft.addons.mcp_servers[0].args[..original.len()],
        original.as_slice()
    );
    assert_eq!(
        draft.addons.mcp_servers[0].environment,
        baseline.addons.mcp_servers[0].environment
    );
    draft
        .set_mcp_text(0, McpText::Args, &serde_json::to_string(original).unwrap())
        .unwrap();
    assert!(!draft.is_dirty(&baseline));
}

#[test]
fn editing_neighbor_environment_preserves_prior_values_and_can_be_undone() {
    let baseline = baseline();
    let mut draft = baseline.clone();
    let original = &baseline.addons.mcp_servers[0].environment.env;
    let mut env = original.clone();
    env.insert("EXTRA".into(), "new".into());
    draft
        .set_mcp_text(0, McpText::Env, &serde_json::to_string(&env).unwrap())
        .unwrap();
    for (key, value) in original {
        assert_eq!(
            draft.addons.mcp_servers[0].environment.env.get(key),
            Some(value)
        );
    }
    assert_eq!(
        draft.addons.mcp_servers[0].args,
        baseline.addons.mcp_servers[0].args
    );
    draft
        .set_mcp_text(0, McpText::Env, &serde_json::to_string(original).unwrap())
        .unwrap();
    assert!(!draft.is_dirty(&baseline));
}

#[test]
fn empty_collections_are_explicit_values_not_unset() {
    let baseline = baseline();
    let mut draft = baseline.clone();
    draft.set_mcp_text(0, McpText::Args, "[]").unwrap();
    draft.set_mcp_text(0, McpText::Env, "{}").unwrap();
    let server = &draft.addons.mcp_servers[0];
    assert!(server.args.is_empty());
    assert!(server.environment.env.is_empty());
    assert_eq!(mcp_text(server, McpText::Args), "[]");
    assert_eq!(mcp_text(server, McpText::Env), "{}");
    let saved = draft.changes(&baseline).addon_settings.unwrap();
    assert!(saved.mcp_servers[0].args.is_empty());
    assert!(saved.mcp_servers[0].environment.env.is_empty());
}

#[test]
fn malformed_or_non_string_arguments_keep_the_last_valid_draft() {
    let baseline = baseline();
    for text in [
        "",
        " \n ",
        "null",
        "{}",
        "false",
        "[1]",
        r#"["valid", null]"#,
        r#"["valid"] trailing"#,
        r#""--stdio""#,
        r#"["unterminated"#,
        "--stdio\n--root",
    ] {
        let mut draft = baseline.clone();
        assert!(
            draft.set_mcp_text(0, McpText::Args, text).is_err(),
            "{text:?}"
        );
        assert_eq!(draft, baseline, "{text:?}");
    }
}

#[test]
fn malformed_or_non_string_environment_keeps_the_last_valid_draft() {
    let baseline = baseline();
    for text in [
        "",
        " \n ",
        "null",
        "[]",
        "A=1",
        r#"{"A": 1}"#,
        r#"{"A": null}"#,
        r#"{"A": {}}"#,
        r#"{"A": "valid"} trailing"#,
        "{}{}",
        r#"{"A": "unterminated"#,
        r#"{"": "value"}"#,
        r#"{" \t": "value"}"#,
    ] {
        let mut draft = baseline.clone();
        assert!(
            draft.set_mcp_text(0, McpText::Env, text).is_err(),
            "{text:?}"
        );
        assert_eq!(draft, baseline, "{text:?}");
    }
}

#[test]
fn duplicate_environment_keys_are_rejected_before_map_collection() {
    let baseline = baseline();
    for text in [
        r#"{"A": "first", "A": "second"}"#,
        r#"{"A": "first", "\u0041": "second"}"#,
    ] {
        let mut draft = baseline.clone();
        let error = draft.set_mcp_text(0, McpText::Env, text).unwrap_err();
        assert!(error.contains("A задана дважды"), "{error}");
        assert_eq!(draft, baseline);
    }
}

#[test]
fn environment_keys_are_preserved_not_trimmed_into_duplicates() {
    let mut draft = baseline();
    draft
        .set_mcp_text(0, McpText::Env, r#"{" A ": " spaced ", "A": "plain"}"#)
        .unwrap();
    let env = &draft.addons.mcp_servers[0].environment.env;
    assert_eq!(env.get(" A ").map(String::as_str), Some(" spaced "));
    assert_eq!(env.get("A").map(String::as_str), Some("plain"));
    let text = mcp_text(&draft.addons.mcp_servers[0], McpText::Env);
    let before = draft.clone();
    draft.set_mcp_text(0, McpText::Env, &text).unwrap();
    assert_eq!(draft, before);
}

#[test]
fn other_field_edits_do_not_reparse_exact_collections() {
    let baseline = baseline();
    let mut draft = baseline.clone();
    draft.set_mcp_text(0, McpText::TimeoutMs, "500").unwrap();
    draft
        .set_mcp_text(0, McpText::EnvAllowlist, " PATH \n\n HOME ")
        .unwrap();
    let before = &baseline.addons.mcp_servers[0];
    let after = &draft.addons.mcp_servers[0];
    assert_eq!(after.args, before.args);
    assert_eq!(after.environment.env, before.environment.env);
    assert_eq!(after.environment.env_allowlist, ["PATH", "HOME"]);
    assert_eq!(after.timeout_ms, Some(500));
}

#[test]
fn changed_collections_reach_the_existing_save_dto_without_another_codec() {
    let baseline = baseline();
    let mut draft = baseline.clone();
    let mut args = baseline.addons.mcp_servers[0].args.clone();
    args.push("last".into());
    draft
        .set_mcp_text(0, McpText::Args, &serde_json::to_string(&args).unwrap())
        .unwrap();
    let saved = draft.changes(&baseline).addon_settings.unwrap();
    let decoded: proteus_contracts::app_protocol::addons::AppAddonsUpdate =
        serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
    assert_eq!(decoded, saved);
    assert_eq!(decoded.mcp_servers[0], draft.addons.mcp_servers[0]);
    assert_eq!(
        decoded.mcp_servers[0].environment.env,
        baseline.addons.mcp_servers[0].environment.env
    );
}
