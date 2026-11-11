//! Check the authored JSON at its actual schema/rendering boundary.
//! Expected colors come from the source being loaded, not a second palette table.

use gpui_kit::Hsla;
use gpui_kit::component::{
    Colorize, Theme, ThemeSet, ThemeToken, black, neutral_50, neutral_100, neutral_200,
    neutral_300, neutral_400, neutral_600, neutral_700, neutral_800, neutral_900, neutral_950,
    white,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, rc::Rc};

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

/// Roles consumed by Proteus surfaces and controls. Sharing this boundary
/// with the protected-payload check prevents silently broadening adaptations.
fn explicit_roles(theme: &Theme) -> impl Iterator<Item = (&'static str, Hsla, ThemeToken)> {
    [
        (
            "title_bar.background",
            theme.title_bar,
            theme.tokens.title_bar,
        ),
        (
            "status_bar.background",
            theme.status_bar,
            theme.tokens.status_bar,
        ),
        ("sidebar.background", theme.sidebar, theme.tokens.sidebar),
        ("tab_bar.background", theme.tab_bar, theme.tokens.tab_bar),
        (
            "tab_bar.segmented.background",
            theme.tab_bar_segmented,
            theme.tokens.tab_bar_segmented,
        ),
        ("background", theme.background, theme.tokens.background),
        (
            "tab.active.background",
            theme.tab_active,
            theme.tokens.tab_active,
        ),
        (
            "group_box.background",
            theme.group_box,
            theme.tokens.group_box,
        ),
        ("popover.background", theme.popover, theme.tokens.popover),
        ("muted.background", theme.muted, theme.tokens.muted),
        ("border", theme.border, theme.tokens.border),
        (
            "accordion.background",
            theme.accordion,
            theme.tokens.accordion,
        ),
        (
            "secondary.background",
            theme.secondary,
            theme.tokens.secondary,
        ),
        (
            "button.secondary.background",
            theme.button_secondary,
            theme.tokens.button_secondary,
        ),
        ("foreground", theme.foreground, theme.tokens.foreground),
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
        (
            "chat.secondary.foreground",
            theme.chat_secondary_foreground,
            theme.tokens.chat_secondary_foreground,
        ),
        (
            "muted.foreground",
            theme.muted_foreground,
            theme.tokens.muted_foreground,
        ),
        (
            "tab.foreground",
            theme.tab_foreground,
            theme.tokens.tab_foreground,
        ),
        (
            "list.hover.background",
            theme.list_hover,
            theme.tokens.list_hover,
        ),
        (
            "secondary.hover.background",
            theme.secondary_hover,
            theme.tokens.secondary_hover,
        ),
        (
            "secondary.active.background",
            theme.secondary_active,
            theme.tokens.secondary_active,
        ),
        (
            "button.secondary.hover.background",
            theme.button_secondary_hover,
            theme.tokens.button_secondary_hover,
        ),
        (
            "button.secondary.active.background",
            theme.button_secondary_active,
            theme.tokens.button_secondary_active,
        ),
    ]
    .into_iter()
}

#[::core::prelude::v1::test]
fn explicit_builtin_roles_survive_schema_and_theme_resolution() {
    let mut seen = BTreeSet::new();
    for source in sources() {
        let raw: Value = serde_json::from_str(source).unwrap();
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for (raw, config) in raw["themes"].as_array().unwrap().iter().zip(set.themes) {
            let name = config.name.to_string();
            assert!(seen.insert(name.clone()), "duplicate builtin {name}");
            let encoded = serde_json::to_value(&config).unwrap();
            let dark = config.mode.is_dark();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            for (field, color, token) in explicit_roles(&theme) {
                let Some(value) = raw["colors"].get(field) else {
                    // Unconfigured roles intentionally follow schema fallbacks.
                    continue;
                };
                assert_eq!(
                    &encoded["colors"][field], value,
                    "schema ignored {name}: {field}"
                );
                let expected = expected_color(value.as_str().unwrap());
                assert_eq!(color, expected, "{name}: {field}");
                assert_eq!(token.color, expected, "{name}: token {field}");
                if field != "foreground" && !field.ends_with(".foreground") {
                    assert_eq!(
                        token.background,
                        expected.into(),
                        "{name}: background token {field}"
                    );
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
    // One inventory assertion protects missing/duplicate bundled variants.
    assert_eq!(seen.len(), 20);
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
        explicit_roles(&Theme::default())
            .map(|(field, _, _)| field)
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
