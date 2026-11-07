use gpui_kit::Hsla;
use gpui_kit::component::{
    Colorize, Theme, ThemeSet, black, neutral_100, neutral_200, neutral_600, neutral_800,
    neutral_900, neutral_950, white,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, rc::Rc};

// Explicit source decisions, independent of the metadata used to audit them.
// name, chrome, sidebar, content, card, popup, muted, border.
const EXPECTED: &[(&str, [&str; 7])] = &[
    (
        "Catppuccin Latte",
        [
            "#dce0e8", "#e6e9ef", "#eff1f5", "#ccd0da", "#ccd0da", "#ccd0da", "#acb0be",
        ],
    ),
    (
        "Catppuccin Frappe",
        [
            "#232634", "#292c3c", "#303446", "#414559", "#414559", "#414559", "#626880",
        ],
    ),
    (
        "Catppuccin Macchiato",
        [
            "#181926", "#1e2030", "#24273a", "#363a4f", "#363a4f", "#363a4f", "#5b6078",
        ],
    ),
    (
        "Catppuccin Mocha",
        [
            "#11111b", "#181825", "#1e1e2e", "#313244", "#313244", "#313244", "#585b70",
        ],
    ),
    (
        "Tokyo Night",
        [
            "#0c0e14", "#16161e", "#1a1b26", "#292e42", "#292e42", "#292e42", "#15161e",
        ],
    ),
    (
        "Tokyo Storm",
        [
            "#1b1e2d", "#1f2335", "#24283b", "#292e42", "#292e42", "#292e42", "#1d202f",
        ],
    ),
    (
        "Tokyo Moon",
        [
            "#191b29", "#1e2030", "#222436", "#2f334d", "#2f334d", "#2f334d", "#1b1d2b",
        ],
    ),
    (
        "Gruvbox Light",
        [
            "#ebdbb2", "#f2e5bc", "#fbf1c7", "#ebdbb2", "#ebdbb2", "#ebdbb2", "#bdae93",
        ],
    ),
    (
        "Gruvbox Dark",
        [
            "#1d2021", "#282828", "#32302f", "#3c3836", "#3c3836", "#3c3836", "#665c54",
        ],
    ),
    (
        "Everforest Light",
        [
            "#efebd4", "#f4f0d9", "#fdf6e3", "#f4f0d9", "#f4f0d9", "#f4f0d9", "#e0dcc7",
        ],
    ),
    (
        "Everforest Dark",
        [
            "#21272b", "#272e33", "#2d353b", "#343f44", "#343f44", "#343f44", "#4f585e",
        ],
    ),
    (
        "Solarized Light",
        [
            "#ddd6c1", "#eee8d5", "#fdf6e3", "#eee8d5", "#eee8d5", "#eee8d5", "#d3af86",
        ],
    ),
    (
        "Solarized Dark",
        [
            "#10192c",
            "#00212b",
            "#002b36",
            "#073642",
            "#073642",
            "#073642",
            "#2aa19899",
        ],
    ),
    (
        "Ayu Light",
        [
            "#ebeef0",
            "#f8f9fa",
            "#fcfcfc",
            "#fafafa",
            "#ffffff",
            "#fafafa",
            "#6b7d8f1f",
        ],
    ),
    (
        "Ayu Dark",
        [
            "#010102", "#0d1017", "#10141c", "#141821", "#0f131a", "#141821", "#1b1f29",
        ],
    ),
    (
        "Proteus Graphite",
        [
            "#131313", "#181818", "#212121", "#282828", "#282828", "#282828", "#414141",
        ],
    ),
    (
        "Proteus Slate",
        [
            "#10141b", "#171d26", "#202630", "#252e3b", "#2c3746", "#283240", "#455166",
        ],
    ),
    (
        "Proteus Light",
        [
            "#f3f4f7", "#f6f7f9", "#ffffff", "#f5f5f5", "white", "#f1f2f5", "#e3e5ea",
        ],
    ),
    (
        "Default Dark",
        [
            "black",
            "neutral-950",
            "neutral-900",
            "neutral-800",
            "neutral-800",
            "neutral-800",
            "neutral-600",
        ],
    ),
    (
        "Default Light",
        [
            "#F8F8F8",
            "#fafafa",
            "white",
            "#f5f5f5",
            "white",
            "neutral-100",
            "neutral-200",
        ],
    ),
];

const FIELDS: &[(&str, usize)] = &[
    ("title_bar.background", 0),
    ("status_bar.background", 0),
    ("sidebar.background", 1),
    ("tab_bar.background", 1),
    ("tab_bar.segmented.background", 1),
    ("background", 2),
    ("tab.active.background", 2),
    ("group_box.background", 3),
    ("popover.background", 4),
    ("muted.background", 5),
    ("border", 6),
];

fn sources() -> impl Iterator<Item = &'static str> {
    super::BUNDLED
        .iter()
        .copied()
        .chain(std::iter::once(include_str!("../proteus.json")))
        .chain(std::iter::once(include_str!(
            "../../../../vendor/gpui-component/src/theme/default-theme.json"
        )))
}

fn expected_color(value: &str) -> Hsla {
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
            let expected = EXPECTED.iter().find(|(n, _)| *n == name).unwrap().1;
            assert!(seen.insert(name.clone()), "duplicate builtin {name}");
            let encoded = serde_json::to_value(&config).unwrap();
            for &(field, role) in FIELDS {
                assert_eq!(raw["colors"][field], expected[role], "{name}: {field}");
                assert_eq!(
                    encoded["colors"][field], expected[role],
                    "schema ignored {name}: {field}"
                );
            }
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            let resolved = [
                (theme.title_bar, theme.tokens.title_bar, 0),
                (theme.status_bar, theme.tokens.status_bar, 0),
                (theme.sidebar, theme.tokens.sidebar, 1),
                (theme.tab_bar, theme.tokens.tab_bar, 1),
                (theme.tab_bar_segmented, theme.tokens.tab_bar_segmented, 1),
                (theme.background, theme.tokens.background, 2),
                (theme.tab_active, theme.tokens.tab_active, 2),
                (theme.group_box, theme.tokens.group_box, 3),
                (theme.popover, theme.tokens.popover, 4),
                (theme.muted, theme.tokens.muted, 5),
                (theme.border, theme.tokens.border, 6),
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
    assert_eq!(role_fields, FIELDS.iter().map(|(name, _)| *name).collect());
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
            assert!(!tokens[role].as_str().unwrap().is_empty());
        }
    }
}
