//! Offline builtin adaptations have explicit inputs, rounding and provenance.
//! Applying a custom theme still goes directly through the existing schema.

use std::collections::BTreeSet;

use gpui_kit::{Hsla, Rgba};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::tests::{expected_color, sources};

const ADAPTATIONS: &str = include_str!("BUILTIN-ADAPTATIONS.json");
const BASELINE: &str = "715b19f10851c4efc666e6e4cb2af1bce923a7a5";

fn bytes(value: &str) -> [f32; 3] {
    let color: Rgba = expected_color(value).into();
    assert_eq!(color.a, 1., "recipe inputs must be opaque");
    [color.r, color.g, color.b].map(|value| (value * 255.).round())
}

fn from_bytes(bytes: [f32; 3]) -> Hsla {
    let [r, g, b] = bytes.map(|value| value.round() as u8);
    expected_color(&format!("#{r:02x}{g:02x}{b:02x}"))
}

fn recipe_color(recipe: &Value) -> Hsla {
    match recipe["kind"].as_str().unwrap() {
        "authored" => expected_color(recipe["reference"].as_str().unwrap()),
        "neutral_composite" | "flattened_legacy_overlay" => {
            let normal = bytes(recipe["normal"].as_str().unwrap());
            let overlay = bytes(recipe["overlay"].as_str().unwrap());
            let alpha = if recipe["kind"] == "neutral_composite" {
                recipe["alpha_percent"].as_f64().unwrap() / 100.
            } else {
                recipe["alpha_byte"].as_f64().unwrap() / 255.
            } as f32;
            from_bytes(std::array::from_fn(|channel| {
                normal[channel] * (1. - alpha) + overlay[channel] * alpha
            }))
        }
        other => panic!("unknown recipe {other}"),
    }
}

#[::core::prelude::v1::test]
fn builtin_control_states_reproduce_their_recorded_offline_recipes() {
    let metadata: Value = serde_json::from_str(ADAPTATIONS).unwrap();
    let policy = &metadata["control_state_policy"];
    assert_eq!(policy["baseline_commit"], BASELINE);
    let records = policy["variants"].as_array().unwrap();
    assert_eq!(records.len(), 20);
    let mut seen = BTreeSet::new();
    for source in sources() {
        let source: Value = serde_json::from_str(source).unwrap();
        for theme in source["themes"].as_array().unwrap() {
            let name = theme["name"].as_str().unwrap();
            let recipe = records
                .iter()
                .find(|record| record["name"] == name)
                .unwrap();
            assert!(seen.insert(name.to_owned()));
            let colors = &theme["colors"];
            assert_eq!(
                colors["secondary.background"], recipe["normal"],
                "{name}: original fill changed"
            );
            assert_eq!(
                colors["button.secondary.background"], recipe["normal"],
                "{name}: button normal drift"
            );
            for (state, field) in [("hover", "hover"), ("pressed", "active")] {
                let expected = recipe_color(&recipe[state]);
                assert_eq!(
                    expected,
                    expected_color(recipe[state]["color"].as_str().unwrap())
                );
                for family in ["secondary", "button.secondary"] {
                    assert_eq!(
                        expected_color(
                            colors[format!("{family}.{field}.background")]
                                .as_str()
                                .unwrap()
                        ),
                        expected,
                        "{name}: recipe drift for {family}/{state}"
                    );
                }
                assert!(!recipe[state]["source"].as_str().unwrap().is_empty());
            }
            let primary = colors
                .as_object()
                .unwrap()
                .iter()
                .filter(|(field, _)| {
                    field.starts_with("primary.")
                        || field.starts_with("button.primary.")
                        || field.starts_with("sidebar.primary.")
                })
                .map(|(field, value)| (field.clone(), value.clone()))
                .collect::<serde_json::Map<_, _>>();
            let mut primary = Value::Object(primary);
            primary.sort_all_objects();
            let hash = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&primary).unwrap())
            );
            assert_eq!(
                recipe["primary_payload_sha256"], hash,
                "{name}: primary colors changed"
            );
        }
    }
    assert_eq!(seen.len(), records.len());
}

#[::core::prelude::v1::test]
fn dark_content_midpoints_remain_between_the_original_authored_surfaces() {
    let metadata: Value = serde_json::from_str(ADAPTATIONS).unwrap();
    let policy = &metadata["content_policy"];
    assert_eq!(policy["baseline_commit"], BASELINE);
    let records = policy["variants"].as_array().unwrap();
    assert_eq!(records.len(), 13);
    let mut count = 0;
    for source in sources() {
        let source: Value = serde_json::from_str(source).unwrap();
        for theme in source["themes"].as_array().unwrap() {
            if theme["mode"] != "dark" {
                continue;
            }
            let name = theme["name"].as_str().unwrap();
            let recipe = records
                .iter()
                .find(|record| record["name"] == name)
                .unwrap();
            assert_eq!(theme["colors"]["sidebar.background"], recipe["sidebar"]);
            let sidebar = bytes(recipe["sidebar"].as_str().unwrap());
            let original = bytes(recipe["original_content"].as_str().unwrap());
            assert_eq!(
                from_bytes(sidebar),
                expected_color(recipe["sidebar_srgb_hex"].as_str().unwrap())
            );
            assert_eq!(
                from_bytes(original),
                expected_color(recipe["original_content_srgb_hex"].as_str().unwrap())
            );
            let expected = from_bytes(std::array::from_fn(|channel| {
                (sidebar[channel] + original[channel]) / 2.
            }));
            assert_eq!(
                expected_color(recipe["content"].as_str().unwrap()),
                expected
            );
            assert_eq!(
                expected_color(theme["colors"]["background"].as_str().unwrap()),
                expected
            );
            count += 1;
        }
    }
    assert_eq!(count, records.len());
}
