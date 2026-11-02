use super::*;

#[test]
fn saved_chat_areas_restore_together_only_in_their_namespace() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = crate::launch::Target::Spawn {
        profile: Some("areas".into()),
    };
    let mut prefs = Preferences::default();
    prefs.tabs_workspace = TabWorkspace::new(workspace, &target);
    prefs.tabs.select("chat:/first");
    let mut first = prefs.tabs.active_layout().clone();
    first.open("files:first.rs", |_| true);
    prefs.tabs.save_active(first.clone());
    prefs.tabs.select("codex:second");
    let encoded = serde_json::to_string(&prefs).unwrap();
    let restored = parse(&encoded, Path::new("fixture.json")).unwrap();
    assert_eq!(
        serde_json::to_value(&restored.tabs).unwrap(),
        serde_json::to_value(&prefs.tabs).unwrap()
    );
    assert_eq!(
        restored
            .tabs_for(workspace, &target)
            .chats()
            .collect::<Vec<_>>(),
        ["codex:second"]
    );
    let mut areas = restored.spaces_for(workspace, &target);
    areas.select("chat:/first");
    assert_eq!(
        serde_json::to_value(areas.active_layout()).unwrap(),
        serde_json::to_value(&first).unwrap()
    );
    let other = crate::launch::Target::Spawn {
        profile: Some("other".into()),
    };
    assert!(restored.spaces_for(workspace, &other).is_empty());
    assert!(
        restored
            .spaces_for(workspace.parent().unwrap(), &target)
            .is_empty()
    );
    assert!(
        restored
            .spaces_for(
                workspace,
                &crate::launch::Target::Attach {
                    origin: "http://localhost:9".into(),
                    token: "unpersisted".into()
                }
            )
            .is_empty()
    );
}

#[test]
fn saved_legacy_layout_and_hints_upgrade_once_without_losing_tabs() {
    let mut legacy = TabLayout::default();
    legacy.open("chat:/root", |_| false);
    legacy.open("codex:child", |_| false);
    legacy.open("files:main.rs", |_| true);
    legacy.keep("files:main.rs");
    legacy.set_pinned("files:main.rs", true);
    let text = serde_json::to_string(&serde_json::json!({
        "tabs": legacy,
        "hints": "keys",
        "motion": "reduced"
    }))
    .unwrap();
    // The public typed wire format accepts only the canonical current shape.
    assert!(serde_json::from_str::<Preferences>(&text).is_err());
    let saved = parse(&text, Path::new("legacy.json")).unwrap();
    assert_eq!(saved.tabs.active_root(), Some("codex:child"));
    assert_eq!(
        serde_json::to_value(saved.tabs.active_layout()).unwrap(),
        serde_json::to_value(&legacy).unwrap()
    );
    assert_eq!(saved.motion, MotionPreference::Reduced);
    assert_eq!(saved.hints.title, TitleHint::Never);
    assert!(saved.hints.shortcut);
    let canonical = serde_json::to_value(&saved).unwrap();
    assert!(canonical["tabs"].get("root").is_none());
    assert!(canonical["tabs"]["roots"].get("codex:child").is_some());
    let reopened: Preferences = serde_json::from_value(canonical).unwrap();
    assert_eq!(reopened.tabs, saved.tabs);
}

#[test]
fn legacy_document_only_layout_remains_unowned() {
    let mut legacy = TabLayout::default();
    legacy.open("files:unowned", |_| false);
    let text = serde_json::to_string(&serde_json::json!({ "tabs": legacy })).unwrap();
    let saved = parse(&text, Path::new("legacy.json")).unwrap();
    assert_eq!(saved.tabs.active_root(), None);
    assert_eq!(
        serde_json::to_value(saved.tabs.active_layout()).unwrap(),
        serde_json::to_value(&legacy).unwrap()
    );
}

#[test]
fn invalid_legacy_tree_and_unknown_fields_are_not_erased_by_migration() {
    let mut invalid = serde_json::to_value(TabLayout::default()).unwrap();
    invalid["focused"] = serde_json::json!(900);
    let text = serde_json::json!({ "tabs": invalid }).to_string();
    assert!(parse(&text, Path::new("invalid.json")).is_err());
    let unknown =
        serde_json::json!({ "tabs": TabLayout::default(), "unrecognized": true }).to_string();
    assert!(parse(&unknown, Path::new("invalid.json")).is_err());
    let dangling = serde_json::json!({ "tabs": { "active": "chat:/missing" } }).to_string();
    assert!(parse(&dangling, Path::new("invalid.json")).is_err());
}
