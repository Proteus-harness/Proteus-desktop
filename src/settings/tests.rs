use std::path::Path;

use proteus_contracts::app_protocol::addons::AppAddonsSnapshot;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModule, ConfigBuilderPlugin, ConfigBuilderSlot, ConfigBuilderSnapshot,
    ConfigBuilderTool, ConfigBuilderToolPack,
};
use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};
use serde_json::json;

use super::addons::{
    mcp_servers, missing_skills, normalized, package_contents, package_state, tool_server,
};
use super::catalog::{
    CLIENT_PAGES, CONNECTION_PAGES, CONTEXT_PROVIDER_SLOT, HOOK_SLOT, MODEL_SLOT, POLICY_SLOT,
    PROFILE_PAGES, PackState, PageId, TOOL_SLOT, area_page, missing_tools, module_info,
    pack_summary, page_title, plural, slot_page, slot_pages, tool_pack,
};
use super::draft::Area;
use super::{Draft, FieldKind, FieldPath, TextFormat, field_kind, format_value, parse_text};

mod connections;
mod mcp_json;
mod models;
mod modes;
mod params;

fn integer(minimum: Option<i64>, maximum: Option<i64>) -> TextFormat {
    TextFormat::Integer { minimum, maximum }
}

#[test]
fn text_values_round_trip_and_validate() {
    assert_eq!(
        parse_text(integer(Some(1), Some(10)), " 7 "),
        Ok(Some(json!(7)))
    );
    assert_eq!(
        parse_text(integer(Some(1), None), "0"),
        Err("Не меньше 1".to_string())
    );
    assert_eq!(
        parse_text(integer(None, None), "x"),
        Err("Нужно целое число".to_string())
    );
    assert_eq!(parse_text(TextFormat::Line, "   "), Ok(None));
    assert_eq!(
        parse_text(TextFormat::Lines, "a\n\n b \n"),
        Ok(Some(json!(["a", "b"])))
    );
    assert_eq!(format_value(TextFormat::Lines, &json!(["a", "b"])), "a\nb");
    assert_eq!(parse_text(TextFormat::Json, "null"), Ok(None));
    assert!(parse_text(TextFormat::Json, "{").is_err());
}

#[test]
fn numeric_fields_preserve_json_integer_spellings_beyond_binary64_precision() {
    for text in [
        "9007199254740993",
        "-9007199254740993",
        "18446744073709551615",
    ] {
        let value = parse_text(TextFormat::Number, text).unwrap().unwrap();
        assert_eq!(value.to_string(), text);
        assert_eq!(format_value(TextFormat::Number, &value), text);
    }
    assert_eq!(parse_text(TextFormat::Number, "1.5"), Ok(Some(json!(1.5))));
    assert_eq!(
        parse_text(TextFormat::Number, "1e3"),
        Ok(Some(json!(1000.0)))
    );
    assert!(parse_text(TextFormat::Number, "NaN").is_err());
}

#[test]
fn changes_send_only_changed_sections() {
    let mut baseline = Draft::default();
    baseline.modules.insert("context".into(), "a".into());
    baseline
        .config
        .entry("context".into())
        .or_default()
        .insert("a".into(), json!({"limit": 1, "keep": true}));
    baseline.hooks = vec!["hook.one".into()];
    baseline.mode = "normal".into();

    let mut draft = baseline.clone();
    assert!(!draft.is_dirty(&baseline));

    let limit = FieldPath::new("context", "a", "limit");
    draft.set_value(&limit, Some(json!(5)));
    draft.set_value(&FieldPath::new("workflow", "b", "x"), None);
    draft.set_module("workflow", "b");
    draft.set_hook("hook.one", false);
    draft.mode = "auto".into();

    let request = draft.changes(&baseline);
    assert_eq!(
        request.modules.get("workflow").map(String::as_str),
        Some("b")
    );
    assert!(!request.modules.contains_key("context"));
    assert_eq!(
        request
            .module_config
            .get("context")
            .and_then(|slot| slot.get("a")),
        Some(&json!({"limit": 5, "keep": true}))
    );
    assert!(!request.module_config.contains_key("workflow"));
    assert_eq!(request.hooks, Some(Vec::new()));
    assert!(request.tools_enabled.is_none());
    assert!(request.permission_mode.is_some());

    draft.set_value(&limit, Some(json!(1)));
    draft.set_module("workflow", "a");
    draft.set_module("workflow", "b");
    assert!(draft.changes(&baseline).module_config.is_empty());
}

#[test]
fn reset_leaves_an_empty_module_object_out_of_the_request() {
    let baseline = Draft::default();
    let mut draft = baseline.clone();
    let path = FieldPath::new("tool", "memory", "path");
    draft.set_value(&path, Some(json!("x.jsonl")));
    assert!(draft.is_dirty(&baseline));
    draft.set_value(&path, None);
    assert!(!draft.is_dirty(&baseline));
}

fn module(id: &str, slot: &str, fields: Vec<ConfigField>) -> ConfigBuilderModule {
    ConfigBuilderModule {
        id: id.into(),
        slot: slot.into(),
        active: true,
        source: String::new(),
        version: String::new(),
        api_version: String::new(),
        capabilities: Vec::new(),
        description: None,
        config_schema: Some(ModuleConfigSchema { fields }),
    }
}

fn slot(id: &str, order: u32, modules: Vec<ConfigBuilderModule>) -> ConfigBuilderSlot {
    ConfigBuilderSlot {
        id: id.into(),
        title: id.to_uppercase(),
        responsibility: String::new(),
        active_module: modules.first().map(|module| module.id.clone()),
        required: true,
        category: String::new(),
        order,
        modules,
    }
}

fn tool(name: &str, runtime_managed: bool) -> ConfigBuilderTool {
    ConfigBuilderTool {
        name: name.into(),
        source: "process".into(),
        owner: None,
        safety: "ReadOnly".into(),
        description: String::new(),
        enabled: true,
        runtime_managed,
        registered: true,
    }
}

#[test]
fn pages_cover_known_slots_and_append_unknown_ones() {
    let trigger = ConfigField::new("trigger", "Порог", "", ConfigValueSchema::integer(1));
    let snapshot = ConfigBuilderSnapshot {
        slots: vec![
            slot(
                "workflow",
                2,
                vec![module("codex", "workflow", vec![trigger])],
            ),
            slot("later", 9, Vec::new()),
            slot("earlier", 7, Vec::new()),
            slot(POLICY_SLOT, 8, Vec::new()),
            slot(MODEL_SLOT, 4, Vec::new()),
            slot(HOOK_SLOT, 6, Vec::new()),
            slot(TOOL_SLOT, 5, Vec::new()),
            slot(
                "session_storage",
                10,
                vec![module("jsonl", "session_storage", vec![])],
            ),
        ],
        ..ConfigBuilderSnapshot::default()
    };
    let pages = slot_pages(Some(&snapshot));
    assert_eq!(
        pages,
        [
            PageId::Slot("workflow".into()),
            PageId::Slot("context".into()),
            PageId::Modes,
            PageId::Hooks,
            PageId::Slot("session_storage".into()),
            PageId::Slot("earlier".into()),
            PageId::Slot("later".into()),
        ]
    );
    for page in [
        PageId::Providers,
        PageId::Models,
        PageId::Tools,
        PageId::Catalog,
    ] {
        assert!(CONNECTION_PAGES.contains(&page));
    }
    assert!(CONNECTION_PAGES.iter().all(|page| !pages.contains(page)));
    assert!(PROFILE_PAGES.contains(&PageId::Configurations));
    assert!(CLIENT_PAGES.contains(&PageId::Appearance));
    assert_eq!(slot_page(POLICY_SLOT), PageId::Modes);
    assert_eq!(area_page(&Area::Mode), PageId::Modes);
    assert!(!PageId::Configurations.is_agent());
    assert!(!pages.contains(&PageId::Configurations));
    assert_eq!(
        slot_page(CONTEXT_PROVIDER_SLOT),
        PageId::Slot("context".into())
    );
    assert_eq!(
        page_title(Some(&snapshot), &PageId::Slot("later".into())),
        "LATER"
    );
    assert_eq!(
        page_title(Some(&snapshot), &slot_page("session_storage")),
        "Session Storage"
    );
    let during_load = slot_pages(None);
    assert!(during_load.contains(&slot_page("session_storage")));
    assert!(pages.starts_with(&during_load));
    assert!(
        module_info(&snapshot, "workflow", "codex")
            .and_then(|info| info.schema)
            .is_some()
    );
}

#[test]
fn hook_enable_disable_preserves_order_and_areas_follow_edits() {
    let mut baseline = Draft::default();
    baseline.hooks = vec!["a".into(), "b".into(), "c".into()];
    let mut draft = baseline.clone();
    draft.set_hook("b", false);
    assert_eq!(draft.hooks, ["a", "c"]);
    draft.set_hook("b", true);
    draft.set_hook("b", true);
    assert_eq!(draft.hooks, ["a", "c", "b"]);
    draft.set_value(
        &FieldPath::new("model", "openai", "effort"),
        Some(json!("high")),
    );
    draft.set_tool("grep", true);
    assert_eq!(
        draft.areas(&baseline),
        [Area::Tools, Area::Hooks, Area::Slot("model".into())]
    );
}

#[test]
fn pack_switch_leaves_runtime_tools_alone() {
    let snapshot = ConfigBuilderSnapshot {
        tools: vec![
            tool("grep", false),
            tool("spawn_agent", true),
            tool("shell", false),
        ],
        ..ConfigBuilderSnapshot::default()
    };
    let pack = ConfigBuilderToolPack {
        id: "pack".into(),
        tools: vec!["grep".into(), "spawn_agent".into(), "shell".into()],
    };
    let mut draft = Draft::from_snapshot(&snapshot);
    draft.set_tool("grep", true);
    let summary = pack_summary(&snapshot, &draft, &pack);
    assert_eq!(summary.state, PackState::Mixed);
    assert_eq!(summary.switchable, ["grep", "shell"]);
    assert_eq!((summary.enabled, summary.total), (2, 3));
    assert_eq!(plural(3, "пакет", "пакета", "пакетов"), "3 пакета");
    assert_eq!(plural(11, "пакет", "пакета", "пакетов"), "11 пакетов");
}

fn addons_snapshot() -> AppAddonsSnapshot {
    let skill = |id: &str| {
        json!({
            "id": id, "name": id, "description": "", "path": format!("/skills/{id}"),
            "source": "project", "enabled": true,
        })
    };
    let server = |name: &str, tool: &str| json!({ "name": name, "enabled": true, "tools": [tool], "error": null });
    serde_json::from_value(json!({
        "reload_error": null,
        "writable": true,
        "settings": {
            "addons": {
                "disabled_skills": [],
                "disabled_mcp_servers": [],
                "plugins": [{ "path": "pkg", "enabled": true }],
            },
            "mcp_servers": [{ "name": "db", "command": "db-mcp" }],
        },
        "catalogs": [{
            "provider": "skills",
            "catalog": { "skills": [skill("review"), skill("team:deploy")], "warnings": [] },
            "error": null,
        }],
        "mcp_servers": [server("db", "query"), server("team:x", "x_run")],
        "plugins": [{
            "path": "/work/pkg", "name": "team", "version": "1.0", "description": null,
            "enabled": true, "warnings": [], "error": null,
        }],
    }))
    .expect("addons snapshot")
}

#[test]
fn addon_switches_stay_sets_and_mark_their_pages() {
    let mut baseline = Draft::default();
    baseline.addons = normalized(
        serde_json::from_value(json!({
            "addons": {
                "disabled_skills": ["b", "a", "b"],
                "disabled_mcp_servers": [],
                "plugins": [{ "path": "pkg", "enabled": true }],
            },
            "mcp_servers": [{ "name": "db", "command": "db-mcp", "enabled": false }],
        }))
        .expect("addon settings"),
    );
    assert_eq!(baseline.addons.addons.disabled_skills, ["a", "b"]);
    let mut draft = baseline.clone();
    draft.set_skill("a", true);
    draft.set_skill("a", false);
    assert!(!draft.is_dirty(&baseline));

    assert!(!draft.mcp_enabled("db"));
    draft.set_mcp("db", true);
    draft.set_mcp("team:x", false);
    draft.set_skill("c", false);
    draft.set_package(Path::new("pkg"), false);
    assert!(draft.mcp_enabled("db") && !draft.mcp_enabled("team:x"));
    assert_eq!(
        draft.areas(&baseline),
        [Area::Mcp, Area::Skills, Area::Packages]
    );
}

#[test]
fn addons_snapshot_links_servers_tools_skills_and_packages() {
    let addons = addons_snapshot();
    let mut draft = Draft::default();
    draft.addons.mcp_servers = serde_json::from_value(json!([
        { "name": "db", "command": "db-mcp" },
        { "name": "fresh", "command": "fresh-mcp" },
    ]))
    .expect("servers");
    draft.addons.addons.disabled_skills = vec!["gone".into(), "review".into()];

    let servers = mcp_servers(&draft, Some(&addons));
    let origins: Vec<_> = servers
        .iter()
        .map(|server| {
            (
                server.name,
                server.configured.is_some(),
                server.state.is_some(),
                server.package,
            )
        })
        .collect();
    assert_eq!(
        origins,
        [
            ("db", true, true, None),
            ("fresh", true, false, None),
            ("team:x", false, true, Some("team")),
        ]
    );
    assert_eq!(tool_server(&addons, "x_run"), Some("team:x"));
    assert_eq!(missing_skills(&draft, &addons), ["gone"]);
    assert_eq!(package_contents(&addons, "team"), (1, 1));
    let state = package_state(&addons, Path::new("pkg")).expect("package state");
    assert_eq!(state.name.as_deref(), Some("team"));
}

#[test]
fn nested_values_create_and_prune_their_objects() {
    let object = ConfigField::new(
        "compress",
        "Сжатие",
        "",
        ConfigValueSchema::Object {
            fields: vec![ConfigField::new(
                "mode",
                "Режим",
                "",
                ConfigValueSchema::text(),
            )],
        },
    );
    assert!(matches!(field_kind(&object), FieldKind::Group(fields) if fields.len() == 1));
    let empty = ConfigField::new(
        "meta",
        "Метаданные",
        "",
        ConfigValueSchema::Object { fields: Vec::new() },
    );
    assert!(matches!(
        field_kind(&empty),
        FieldKind::Text(TextFormat::Json)
    ));
    let limit =
        ConfigField::new("limit", "Предел", "", ConfigValueSchema::Json {}).with_default(100000);
    assert!(matches!(
        field_kind(&limit),
        FieldKind::Text(TextFormat::Value)
    ));
    assert_eq!(parse_text(TextFormat::Value, "80%"), Ok(Some(json!("80%"))));
    assert_eq!(
        parse_text(TextFormat::Value, "100000"),
        Ok(Some(json!(100000)))
    );
    assert_eq!(format_value(TextFormat::Value, &json!("80%")), "80%");
    assert_eq!(format_value(TextFormat::Value, &json!("5")), "\"5\"");

    let mut draft = Draft::default();
    let group = FieldPath::new("hook", "dcp", "compress");
    let mode = group.child("mode");
    let turns = FieldPath::new("hook", "dcp", "strategies")
        .child("purgeErrors")
        .child("turns");
    draft.set_value(&mode, Some(json!("message")));
    draft.set_value(&turns, Some(json!(3)));
    assert_eq!(
        draft.module_config("hook", "dcp"),
        Some(&json!({
            "compress": { "mode": "message" },
            "strategies": { "purgeErrors": { "turns": 3 } },
        }))
    );
    assert_eq!(draft.value(&mode), Some(&json!("message")));
    assert!(mode.starts_with(&group) && !group.starts_with(&mode));
    assert_eq!(turns.dotted(), "strategies.purgeErrors.turns");

    draft.set_value(&turns, None);
    assert_eq!(
        draft.module_config("hook", "dcp"),
        Some(&json!({ "compress": { "mode": "message" } }))
    );
    draft.set_value(&group, None);
    assert_eq!(draft.module_config("hook", "dcp"), Some(&json!({})));
}

#[test]
fn tool_edits_and_pack_parameters_share_the_tools_page() {
    assert_eq!(area_page(&Area::Tools), PageId::Tools);
    assert_eq!(slot_page("tool"), PageId::Tools);
    assert_eq!(area_page(&Area::Slot("tool".into())), PageId::Tools);
}

#[test]
fn canonical_pack_uses_the_owner_and_keeps_unplaced_and_missing_tools() {
    use proteus_contracts::contracts::ProcessToolOwner;

    let plugin = |id: &str, names: &[&str]| ConfigBuilderPlugin {
        id: id.into(),
        command: "module-host".into(),
        description: None,
        icon: None,
        exports: Vec::new(),
        tool_packs: vec![ConfigBuilderToolPack {
            id: "tools".into(),
            tools: names.iter().map(|name| (*name).into()).collect(),
        }],
    };
    let mut owned = tool("owned", false);
    owned.owner = Some(ProcessToolOwner {
        component_id: "second".into(),
        module_id: "tools".into(),
    });
    let mut stale = tool("stale_owner", false);
    stale.owner = Some(ProcessToolOwner {
        component_id: "gone".into(),
        module_id: "tools".into(),
    });
    let snapshot = ConfigBuilderSnapshot {
        plugins: vec![
            plugin("first", &["ordinary", "owned", "stale_owner", "missing"]),
            plugin("second", &["ordinary", "owned"]),
        ],
        tools: vec![tool("ordinary", false), owned, stale, tool("loose", false)],
        ..ConfigBuilderSnapshot::default()
    };
    assert_eq!(
        tool_pack(&snapshot, "ordinary").map(|(plugin, _)| plugin.id.as_str()),
        Some("first")
    );
    assert_eq!(
        tool_pack(&snapshot, "owned").map(|(plugin, _)| plugin.id.as_str()),
        Some("second")
    );
    assert!(tool_pack(&snapshot, "stale_owner").is_none());
    assert!(tool_pack(&snapshot, "loose").is_none());
    assert!(tool_pack(&snapshot, "missing").is_none());
    let mut draft = Draft::default();
    draft.set_tool("missing", true);
    assert_eq!(missing_tools(&snapshot, &draft), ["missing"]);
}
