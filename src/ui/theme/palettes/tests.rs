use gpui_kit::Hsla;
use gpui_kit::component::{
    Colorize, Theme, ThemeSet, black, neutral_50, neutral_100, neutral_200, neutral_300,
    neutral_400, neutral_600, neutral_700, neutral_800, neutral_900, neutral_950, white,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, rc::Rc};

use super::fixtures::EXPECTED;

const FIELDS: &[(&str, usize)] = &[
    ("title_bar.background", 0),
    ("status_bar.background", 0),
    ("sidebar.background", 1),
    ("tab_bar.background", 1),
    ("tab_bar.segmented.background", 1),
    ("background", 2),
    ("tab.active.background", 7),
    ("group_box.background", 3),
    ("popover.background", 4),
    ("muted.background", 5),
    ("border", 6),
    ("accordion.background", 8),
    ("secondary.background", 9),
];

pub(super) fn sources() -> impl Iterator<Item = &'static str> {
    super::BUNDLED
        .iter()
        .copied()
        .chain(std::iter::once(include_str!("../proteus.json")))
        .chain(std::iter::once(include_str!(
            "../../../../vendor/gpui-component/src/theme/default-theme.json"
        )))
}

pub(super) fn expected_color(value: &str) -> Hsla {
    // Native named colors use rounded HSL coordinates, not an exact 8-bit
    // hex conversion. Keep exact comparisons against the native token while
    // preserving exact hex assertions for the imported/Proteus surfaces.
    match value {
        "white" => white(),
        "black" => black(),
        "neutral-950" => neutral_950(),
        "neutral-900" => neutral_900(),
        "neutral-800" => neutral_800(),
        "neutral-100" => neutral_100(),
        "neutral-200" => neutral_200(),
        "neutral-600" => neutral_600(),
        "neutral-50" => neutral_50(),
        "neutral-300" => neutral_300(),
        "neutral-400" => neutral_400(),
        "neutral-700" => neutral_700(),
        color => Hsla::parse_hex(color).unwrap(),
    }
}

#[::core::prelude::v1::test]
fn explicit_builtin_roles_survive_schema_and_theme_resolution() {
    let mut seen = BTreeSet::new();
    for source in sources() {
        let raw: Value = serde_json::from_str(source).unwrap();
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for (raw, config) in raw["themes"].as_array().unwrap().iter().zip(set.themes) {
            let name = config.name.to_string();
            let fixture = EXPECTED
                .iter()
                .find(|expected| expected.name == name)
                .unwrap();
            let expected = fixture.backgrounds;
            assert!(seen.insert(name.clone()), "duplicate builtin {name}");
            let encoded = serde_json::to_value(&config).unwrap();
            for &(field, role) in FIELDS {
                assert_eq!(raw["colors"][field], expected[role], "{name}: {field}");
                assert_eq!(
                    encoded["colors"][field], expected[role],
                    "schema ignored {name}: {field}"
                );
            }
            let dark = config.mode.is_dark();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            let resolved = [
                (theme.title_bar, theme.tokens.title_bar, 0),
                (theme.status_bar, theme.tokens.status_bar, 0),
                (theme.sidebar, theme.tokens.sidebar, 1),
                (theme.tab_bar, theme.tokens.tab_bar, 1),
                (theme.tab_bar_segmented, theme.tokens.tab_bar_segmented, 1),
                (theme.background, theme.tokens.background, 2),
                (theme.tab_active, theme.tokens.tab_active, 7),
                (theme.group_box, theme.tokens.group_box, 3),
                (theme.popover, theme.tokens.popover, 4),
                (theme.muted, theme.tokens.muted, 5),
                (theme.border, theme.tokens.border, 6),
                (theme.accordion, theme.tokens.accordion, 8),
                (theme.secondary, theme.tokens.secondary, 9),
            ];
            for (color, token, role) in resolved {
                let expected = expected_color(expected[role]);
                assert_eq!(color, expected, "{name}: role {role}");
                assert_eq!(token.color, expected, "{name}: token {role}");
                assert_eq!(
                    token.background,
                    expected.into(),
                    "{name}: background token {role}"
                );
            }
            for (field, color, token, value) in [
                (
                    "list.hover.background",
                    theme.list_hover,
                    theme.tokens.list_hover,
                    fixture.hover_overlay,
                ),
                (
                    "foreground",
                    theme.foreground,
                    theme.tokens.foreground,
                    fixture.foreground,
                ),
                (
                    "muted.foreground",
                    theme.muted_foreground,
                    theme.tokens.muted_foreground,
                    fixture.muted,
                ),
                (
                    "tab.foreground",
                    theme.tab_foreground,
                    theme.tokens.tab_foreground,
                    fixture.tab_foreground,
                ),
            ] {
                assert_eq!(raw["colors"][field], value, "{name}: {field}");
                assert_eq!(
                    encoded["colors"][field], value,
                    "schema ignored {name}: {field}"
                );
                let expected = expected_color(value);
                assert_eq!(color, expected, "{name}: {field}");
                assert_eq!(token.color, expected, "{name}: token {field}");
            }
            if fixture.imported {
                for (field, color, token) in [
                    (
                        "sidebar.foreground",
                        theme.sidebar_foreground,
                        theme.tokens.sidebar_foreground,
                    ),
                    (
                        "group_box.foreground",
                        theme.group_box_foreground,
                        theme.tokens.group_box_foreground,
                    ),
                    (
                        "popover.foreground",
                        theme.popover_foreground,
                        theme.tokens.popover_foreground,
                    ),
                    (
                        "secondary.foreground",
                        theme.secondary_foreground,
                        theme.tokens.secondary_foreground,
                    ),
                    (
                        "tab.active.foreground",
                        theme.tab_active_foreground,
                        theme.tokens.tab_active_foreground,
                    ),
                    (
                        "button.secondary.foreground",
                        theme.button_secondary_foreground,
                        theme.tokens.button_secondary_foreground,
                    ),
                ] {
                    assert_eq!(raw["colors"][field], fixture.foreground, "{name}: {field}");
                    assert_eq!(
                        encoded["colors"][field], fixture.foreground,
                        "schema ignored {name}: {field}"
                    );
                    let expected = expected_color(fixture.foreground);
                    assert_eq!(color, expected, "{name}: {field}");
                    assert_eq!(token.color, expected, "{name}: token {field}");
                }
            }
            for (role, other) in [
                ("content", theme.background),
                ("card", theme.group_box),
                ("input", theme.secondary),
            ] {
                assert_ne!(
                    theme.accordion, other,
                    "{name}: disclosure merged with {role}"
                );
            }
            if dark {
                assert_ne!(
                    theme.tab_active, theme.background,
                    "{name}: active tab/content merged"
                );
                assert!(
                    theme.tab_active.l > theme.background.l,
                    "{name}: dark active tab must be lighter than content"
                );
            }
            assert_ne!(
                theme.title_bar, theme.sidebar,
                "{name}: chrome/sidebar merged"
            );
            assert_ne!(
                theme.sidebar, theme.background,
                "{name}: sidebar/content merged"
            );
            assert_ne!(
                theme.title_bar, theme.background,
                "{name}: chrome/content merged"
            );
        }
    }
    assert_eq!(seen.len(), EXPECTED.len());
}

#[::core::prelude::v1::test]
fn structural_adaptation_preserves_all_other_imported_colors_and_highlights() {
    let mapping: Value = serde_json::from_str(include_str!("ROLE-MAPPING.json")).unwrap();
    let variants = mapping["variants"].as_array().unwrap();
    let role_fields: BTreeSet<&str> = mapping["fields"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(|fields| fields.as_array().unwrap())
        .map(|field| field.as_str().unwrap())
        .collect();
    assert_eq!(
        role_fields,
        FIELDS
            .iter()
            .map(|(name, _)| *name)
            .chain([
                "foreground",
                "sidebar.foreground",
                "group_box.foreground",
                "popover.foreground",
                "secondary.foreground",
                "tab.active.foreground",
                "button.secondary.foreground",
                "muted.foreground",
                "tab.foreground",
                "list.hover.background",
            ])
            .collect()
    );
    let mut seen = BTreeSet::new();
    for source in super::BUNDLED {
        let set: Value = serde_json::from_str(source).unwrap();
        for theme in set["themes"].as_array().unwrap() {
            let name = theme["name"].as_str().unwrap();
            let audit = variants.iter().find(|v| v["name"] == name).unwrap();
            let colors: serde_json::Map<String, Value> = theme["colors"]
                .as_object()
                .unwrap()
                .iter()
                .filter(|(key, _)| !role_fields.contains(key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            let mut protected = json!({"colors":colors,"highlight":theme["highlight"]});
            // Match the documented recursively sorted Python canonical JSON,
            // including when another dependency enables preserve_order.
            protected.sort_all_objects();
            let digest = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&protected).unwrap())
            );
            assert_eq!(
                digest, audit["preserved_payload_sha256"],
                "{name}: nonstructural import changed"
            );
            for (role, fields) in mapping["fields"].as_object().unwrap() {
                for field in fields.as_array().unwrap() {
                    assert_eq!(
                        theme["colors"][field.as_str().unwrap()],
                        audit["roles"][role],
                        "{name}: provenance drift"
                    );
                }
            }
            assert!(seen.insert(name.to_owned()));
        }
    }
    assert_eq!(seen.len(), variants.len());
    assert_eq!(seen.len(), 15);
}

#[::core::prelude::v1::test]
fn role_provenance_pins_primary_sources_and_each_family_token() {
    let mapping: Value = serde_json::from_str(include_str!("ROLE-MAPPING.json")).unwrap();
    let sources = mapping["sources"].as_object().unwrap();
    for source in sources.values() {
        let rev = source["commit"].as_str().unwrap();
        assert_eq!(rev.len(), 40);
        assert!(rev.bytes().all(|b| b.is_ascii_hexdigit()));
        let hash = source["sha256"].as_str().unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(
            source["url"]
                .as_str()
                .unwrap()
                .contains(&format!("/blob/{rev}/"))
        );
    }
    for variant in mapping["variants"].as_array().unwrap() {
        let family = &mapping["families"][variant["family"].as_str().unwrap()];
        for source in family["sources"].as_array().unwrap() {
            assert!(sources.contains_key(source.as_str().unwrap()));
        }
        let tokens = &family["tokens"];
        let tokens = if tokens.get("chrome").is_some() {
            tokens
        } else {
            &tokens[variant["profile"].as_str().unwrap()]
        };
        for role in mapping["fields"].as_object().unwrap().keys() {
            let token = variant["tokens"].get(role).unwrap_or(&tokens[role]);
            assert!(
                !token.as_str().unwrap().is_empty(),
                "missing token for {role}"
            );
        }
    }
}
