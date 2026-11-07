//! Readability checks use resolved colors, including native named HSL tokens.
//! These scoped theme assertions do not certify every rendered UI state.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeSet};
use gpui_kit::{Hsla, Rgba};
use serde_json::Value;

use super::tests::{expected_color, sources};

const MINIMUM: f64 = 4.5;

fn composite(foreground: Rgba, background: Rgba) -> Rgba {
    let alpha = foreground.a + background.a * (1. - foreground.a);
    let channel =
        |fg: f32, bg: f32| (fg * foreground.a + bg * background.a * (1. - foreground.a)) / alpha;
    Rgba {
        r: channel(foreground.r, background.r),
        g: channel(foreground.g, background.g),
        b: channel(foreground.b, background.b),
        a: alpha,
    }
}

fn luminance(color: Rgba) -> f64 {
    let linear = |value: f32| {
        let value = f64::from(value);
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

/// Alpha composition occurs on the actual painted sRGB surface before WCAG
/// linearization. Treating translucent foreground RGB as opaque overstates it.
fn contrast(foreground: Hsla, surface: Rgba) -> f64 {
    assert_eq!(
        surface.a, 1.,
        "contrast requires a resolved opaque backdrop"
    );
    let fg = luminance(composite(foreground.into(), surface));
    let bg = luminance(surface);
    (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05)
}

fn surfaces(theme: &Theme) -> [(&'static str, Rgba); 6] {
    let backdrop: Rgba = theme.background.into();
    assert_eq!(backdrop.a, 1., "builtin content must be opaque");
    [
        ("content", theme.background),
        ("sidebar", theme.sidebar),
        ("card", theme.group_box),
        ("input", theme.secondary),
        ("popup", theme.popover),
        ("disclosure/dock", theme.accordion),
    ]
    .map(|(name, fill)| (name, composite(fill.into(), backdrop)))
}

fn assert_readable(theme: &str, role: &str, foreground: Hsla, fill: Rgba) {
    let ratio = contrast(foreground, fill);
    assert!(
        ratio >= MINIMUM,
        "{theme}: {role} contrast {ratio:.4}:1 < {MINIMUM}:1"
    );
}

#[::core::prelude::v1::test]
fn normal_and_muted_labels_meet_minimum_on_their_resolved_surfaces() {
    let mut count = 0;
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            for (surface, fill) in surfaces(&theme) {
                assert_readable(&name, &format!("normal/{surface}"), theme.foreground, fill);
                assert_readable(
                    &name,
                    &format!("muted/{surface}"),
                    theme.muted_foreground,
                    fill,
                );
            }
            let backdrop: Rgba = theme.background.into();
            for (role, foreground, fill) in [
                ("sidebar label", theme.sidebar_foreground, theme.sidebar),
                ("card label", theme.group_box_foreground, theme.group_box),
                ("input label", theme.secondary_foreground, theme.secondary),
                ("popup label", theme.popover_foreground, theme.popover),
                (
                    "active tab label",
                    theme.tab_active_foreground,
                    theme.tab_active,
                ),
                ("inactive tab label", theme.tab_foreground, theme.sidebar),
            ] {
                assert_readable(&name, role, foreground, composite(fill.into(), backdrop));
            }
            // The same native secondary text color covers the normal, hover
            // and active fills. Disabled styling deliberately stays untouched.
            for (state, fill) in [
                ("normal", theme.button_secondary),
                ("hover", theme.button_secondary_hover),
                ("active", theme.button_secondary_active),
            ] {
                for (parent, parent_fill) in [("card", theme.group_box), ("dock", theme.accordion)]
                {
                    let parent_fill = composite(parent_fill.into(), backdrop);
                    let fill = composite(fill.into(), parent_fill);
                    assert_readable(
                        &name,
                        &format!("secondary button {state}/{parent}"),
                        theme.button_secondary_foreground,
                        fill,
                    );
                }
            }
            count += 1;
        }
    }
    assert_eq!(count, 20);
}

#[::core::prelude::v1::test]
fn imported_muted_choice_is_the_first_authored_candidate_that_passes() {
    let mapping: Value = serde_json::from_str(include_str!("ROLE-MAPPING.json")).unwrap();
    for source in super::BUNDLED {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            let variant = mapping["variants"]
                .as_array()
                .unwrap()
                .iter()
                .find(|variant| variant["name"] == name)
                .unwrap();
            let first = variant["muted_candidates"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| {
                    let candidate = expected_color(candidate["color"].as_str().unwrap());
                    surfaces(&theme)
                        .iter()
                        .all(|(_, fill)| contrast(candidate, *fill) >= MINIMUM)
                })
                .unwrap();
            assert_eq!(
                theme.muted_foreground,
                expected_color(first["color"].as_str().unwrap()),
                "{name}: offline selection drift"
            );
        }
    }
}

#[::core::prelude::v1::test]
fn alpha_is_composited_before_luminance_instead_of_assumed_opaque() {
    let white: Rgba = expected_color("white").into();
    assert!((contrast(expected_color("black"), white) - 21.).abs() < 0.0001);
    let translucent = contrast(expected_color("#00000080"), white);
    assert!((translucent - 4.0041).abs() < 0.001);
    assert!(translucent < MINIMUM);
    let ayu = expected_color("#020a12a3");
    assert!(contrast(ayu, expected_color("#cecece").into()) > MINIMUM);
}
