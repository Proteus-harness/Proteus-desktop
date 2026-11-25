//! Readability checks use resolved colors, including native named HSL tokens.
//! These scoped theme assertions do not certify every rendered UI state.

use std::rc::Rc;

use gpui_kit::component::{Theme, ThemeSet};
use gpui_kit::{Hsla, Rgba};

use super::super::surface_palette::SurfacePalette;
use super::tests::{expected_color, sources};

const MINIMUM: f64 = 4.5;

#[::core::prelude::v1::test]
fn danger_button_text_stays_readable_through_hover_and_press() {
    let sparse = r##"{"themes":[{"name":"Dark with a muted red fill","mode":"dark","colors":{
        "background":"#262624","danger.background":"#4b2929","danger.foreground":"#f4a29a"
    }}]}"##;
    for source in sources().chain(std::iter::once(sparse)) {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            for fill in [
                theme.button_danger,
                theme.button_danger_hover,
                theme.button_danger_active,
            ] {
                assert_readable(
                    &name,
                    "danger button",
                    theme.button_danger_foreground,
                    composite(fill.into(), theme.background.into()),
                );
            }
        }
    }
}

#[::core::prelude::v1::test]
fn unchecked_switch_indicator_and_outline_remain_visible_on_panel_levels() {
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            for parent in [theme.background, theme.group_box, theme.accordion] {
                let parent: Rgba = parent.into();
                let track = composite(theme.switch.into(), parent);
                for (role, backdrop) in [("thumb/track", track), ("outline/panel", parent)] {
                    let ratio = contrast(theme.switch_thumb_unchecked, backdrop);
                    assert!(
                        ratio >= 3.,
                        "{name}: unchecked switch {role} contrast {ratio:.4}:1"
                    );
                }
            }
        }
    }
}

#[::core::prelude::v1::test]
fn unchecked_switch_color_can_be_configured_separately() {
    let set: ThemeSet = serde_json::from_value(serde_json::json!({
        "name": "Switch fixture",
        "themes": [{"name": "Switch fixture", "mode": "dark", "colors": {
            "switch.thumb.background": "#ffffff",
            "switch.thumb.unchecked.background": "#a5adcb"
        }}]
    }))
    .unwrap();
    let mut theme = Theme::default();
    theme.apply_config(&Rc::new(set.themes.into_iter().next().unwrap()));
    assert_eq!(theme.switch_thumb, expected_color("#ffffff"));
    assert_eq!(theme.switch_thumb_unchecked, expected_color("#a5adcb"));
    assert_eq!(
        theme.tokens.switch_thumb_unchecked.color,
        theme.switch_thumb_unchecked
    );
}

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

fn surfaces(theme: &Theme) -> [(&'static str, Rgba); 7] {
    let backdrop: Rgba = theme.background.into();
    assert_eq!(backdrop.a, 1., "builtin content must be opaque");
    [
        ("content", theme.background),
        ("sidebar", theme.sidebar),
        ("card", theme.group_box),
        (
            "input dock",
            crate::ui::kit::composer_dock_background(theme),
        ),
        ("composer", crate::ui::kit::composer_background(theme)),
        ("popup", theme.popover),
        ("disclosure", theme.accordion),
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

fn projected_themes() -> impl Iterator<Item = Theme> {
    use crate::preferences::{DEFAULT_DARK_THEME, DEFAULT_LIGHT_THEME};

    let builtins: ThemeSet = serde_json::from_str(super::super::BUNDLED).unwrap();
    let base = |name| {
        Rc::new(
            builtins
                .themes
                .iter()
                .find(|config| config.name == name)
                .unwrap()
                .clone(),
        )
    };
    let policy = SurfacePalette::new(&base(DEFAULT_DARK_THEME), &base(DEFAULT_LIGHT_THEME));
    sources()
        .flat_map(|source| serde_json::from_str::<ThemeSet>(source).unwrap().themes)
        .map(move |config| {
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            policy.apply(&mut theme);
            theme
        })
}

#[::core::prelude::v1::test]
fn light_settings_cards_remain_distinct_from_content() {
    // This is a local visual separation floor for filled settings groups.
    // Text readability has its own threshold and assertions below.
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            if config.mode.is_dark() {
                continue;
            }
            let name = config.name.to_string();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            let ratio = contrast(theme.group_box, theme.background.into());
            assert!(
                ratio >= 1.10,
                "{name}: settings card/content contrast {ratio:.4}:1 < 1.10:1"
            );
        }
    }
}

#[::core::prelude::v1::test]
fn selector_frames_remain_visible_on_cards_and_editable_fills() {
    let mut count = 0;
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            if !matches!(
                config.name.as_ref(),
                "Proteus Light" | "Default Light" | "Tokyo Night" | "Tokyo Storm" | "Tokyo Moon"
            ) {
                continue;
            }
            let name = config.name.to_string();
            let dark = config.mode.is_dark();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            let ratio = contrast(theme.border, theme.group_box.into());
            assert!(
                ratio >= 1.5,
                "{name}: selector frame/card contrast {ratio:.4}:1 < 1.5:1"
            );
            let ratio = contrast(theme.border, theme.input_background().into());
            assert!(
                ratio >= 1.5,
                "{name}: selector frame/input contrast {ratio:.4}:1 < 1.5:1"
            );
            if dark {
                assert!(
                    luminance(theme.border.into()) > luminance(theme.group_box.into()),
                    "{name}: dark selector frame must be lighter than its card"
                );
            }
            count += 1;
        }
    }
    assert_eq!(count, 5);
}

#[::core::prelude::v1::test]
fn neutral_hover_follows_appearance_and_preserves_surface_hierarchy() {
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let dark = config.mode.is_dark();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            for ((lower_name, lower), (upper_name, upper)) in [
                (("chrome", theme.title_bar), ("sidebar", theme.sidebar)),
                (("sidebar", theme.sidebar), ("content", theme.background)),
            ] {
                assert!(
                    luminance(lower.into()) < luminance(upper.into()),
                    "{name}: {lower_name} must be darker than {upper_name}"
                );
            }
            let bases = [theme.background, theme.group_box, theme.accordion];
            let hovered = bases.map(|base| composite(theme.list_hover.into(), base.into()));
            for (index, (base, hovered)) in bases.iter().zip(hovered).enumerate() {
                let change = luminance(hovered) - luminance((*base).into());
                assert!(
                    if dark { change > 0. } else { change < 0. },
                    "{name}: hover direction on surface {index}"
                );
                assert_readable(&name, "normal/hover", theme.foreground, hovered);
            }
            for (a, b) in [(0, 1), (0, 2), (1, 2)] {
                let before = luminance(bases[a].into()) - luminance(bases[b].into());
                let after = luminance(hovered[a]) - luminance(hovered[b]);
                assert!(
                    before * after > 0.,
                    "{name}: hover merged or inverted surfaces {a}/{b}"
                );
            }
        }
    }
}

#[::core::prelude::v1::test]
fn normal_and_muted_labels_meet_minimum_on_their_resolved_surfaces() {
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            let input_fill: Rgba = theme.input_background().into();
            assert_eq!(
                input_fill.a, 1.,
                "{name}: builtin editable fields must be opaque"
            );
            // Like settings cards, editable fields need a visible local edge
            // from their fill on either shared panel level. This is separate
            // from the text/placeholder readability checks below.
            for (level, parent) in [("card", theme.group_box), ("nested", theme.accordion)] {
                let ratio = contrast(theme.input_background(), parent.into());
                assert!(
                    ratio >= 1.10,
                    "{name}: editable field/{level} contrast {ratio:.4}:1 < 1.10:1"
                );
            }
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
            let selected = composite(
                theme.sidebar_accent.into(),
                composite(theme.sidebar.into(), backdrop),
            );
            for (role, foreground) in [
                ("selected sidebar label", theme.sidebar_foreground),
                ("selected muted label", theme.muted_foreground),
            ] {
                assert_readable(&name, role, foreground, selected);
            }
            for (role, foreground, fill) in [
                ("sidebar label", theme.sidebar_foreground, theme.sidebar),
                ("card label", theme.group_box_foreground, theme.group_box),
                ("input label", theme.foreground, theme.input_background()),
                (
                    "input placeholder",
                    theme.muted_foreground,
                    theme.input_background(),
                ),
                ("popup label", theme.popover_foreground, theme.popover),
                ("active tab label", theme.tab_foreground, theme.tab_active),
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
                for (parent, parent_fill) in [
                    ("card", theme.group_box),
                    ("composer", crate::ui::kit::composer_background(&theme)),
                    (
                        "input dock",
                        crate::ui::kit::composer_dock_background(&theme),
                    ),
                ] {
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

#[::core::prelude::v1::test]
fn filled_neutral_control_states_follow_appearance_without_changing_normal_fill() {
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let dark = config.mode.is_dark();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            for (family, [normal, hover, pressed], foreground) in [
                (
                    "secondary",
                    [
                        theme.secondary,
                        theme.secondary_hover,
                        theme.secondary_active,
                    ],
                    theme.secondary_foreground,
                ),
                (
                    "secondary button",
                    [
                        theme.button_secondary,
                        theme.button_secondary_hover,
                        theme.button_secondary_active,
                    ],
                    theme.button_secondary_foreground,
                ),
            ] {
                for fill in [normal, hover, pressed] {
                    assert_eq!(fill.a, 1., "{name}: {family} depends on its parent fill");
                    assert_readable(&name, family, foreground, fill.into());
                }
                let [normal, hover, pressed] =
                    [normal, hover, pressed].map(|fill| luminance(fill.into()));
                assert!(
                    if dark {
                        normal < hover && hover < pressed
                    } else {
                        normal > hover && hover > pressed
                    },
                    "{name}: {family} hover/pressed direction"
                );
            }
            assert_eq!(
                theme.secondary, theme.button_secondary,
                "{name}: normal state drift"
            );
            assert_eq!(
                theme.secondary_hover, theme.button_secondary_hover,
                "{name}: hover state drift"
            );
            assert_eq!(
                theme.secondary_active, theme.button_secondary_active,
                "{name}: pressed state drift"
            );
        }
    }
}

#[::core::prelude::v1::test]
fn operational_chat_captions_stay_readable_and_quieter_than_replies() {
    for source in sources() {
        let set: ThemeSet = serde_json::from_str(source).unwrap();
        for config in set.themes {
            let name = config.name.to_string();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            let backdrop = theme.background.into();
            assert_readable(
                &name,
                "chat secondary",
                theme.chat_secondary_foreground,
                backdrop,
            );
            assert!(
                contrast(theme.chat_secondary_foreground, backdrop)
                    < contrast(theme.foreground, backdrop),
                "{name}: operational captions must be quieter than reply text"
            );
        }
    }
}

#[::core::prelude::v1::test]
fn custom_chat_caption_role_is_optional_and_does_not_change_settings_labels() {
    for explicit in [None, Some("#939ab7")] {
        let mut colors = serde_json::json!({"muted.foreground": "#b8c0e0"});
        if let Some(color) = explicit {
            colors["chat.secondary.foreground"] = color.into();
        }
        let set: ThemeSet = serde_json::from_value(serde_json::json!({
            "name":"Chat caption fixture", "themes":[{
                "name":"Chat caption fixture", "mode":"dark", "colors": colors
            }]
        }))
        .unwrap();
        let mut theme = Theme::default();
        theme.apply_config(&Rc::new(set.themes.into_iter().next().unwrap()));
        assert_eq!(theme.muted_foreground, expected_color("#b8c0e0"));
        assert_eq!(
            theme.chat_secondary_foreground,
            expected_color(explicit.unwrap_or("#b8c0e0"))
        );
        assert_eq!(
            theme.tokens.chat_secondary_foreground.color,
            theme.chat_secondary_foreground
        );
    }
}

#[::core::prelude::v1::test]
fn tool_and_navigation_glyphs_are_readable_in_each_bundled_palette() {
    use gpui_kit::component::IconTone::{Danger, Muted, Normal, Selected, Success, Warning};
    for theme in projected_themes() {
        for tone in [Normal, Muted, Selected, Success, Warning, Danger] {
            let ink = super::super::icons::color(tone, theme.mode);
            for surface in [theme.background, theme.sidebar] {
                let ratio = contrast(ink, composite(surface.into(), theme.background.into()));
                assert!(
                    ratio >= 3.0,
                    "{} {tone:?}: {ratio:.2}:1",
                    theme.theme_name()
                );
            }
        }
    }
}

#[::core::prelude::v1::test]
fn shared_ink_is_readable_on_selected_builtin_surfaces() {
    for theme in projected_themes() {
        let name = theme.theme_name();
        for (surface, fill) in surfaces(&theme) {
            assert_readable(name, &format!("normal/{surface}"), theme.foreground, fill);
            assert_readable(
                name,
                &format!("muted/{surface}"),
                theme.muted_foreground,
                fill,
            );
        }
        assert_readable(
            name,
            "chat secondary",
            theme.chat_secondary_foreground,
            theme.background.into(),
        );
        for (role, ink, fills) in [
            (
                "primary button",
                theme.button_primary_foreground,
                [
                    theme.button_primary,
                    theme.button_primary_hover,
                    theme.button_primary_active,
                ],
            ),
            (
                "danger button",
                theme.button_danger_foreground,
                [
                    theme.button_danger,
                    theme.button_danger_hover,
                    theme.button_danger_active,
                ],
            ),
            (
                "success button",
                theme.button_success_foreground,
                [
                    theme.button_success,
                    theme.button_success_hover,
                    theme.button_success_active,
                ],
            ),
            (
                "info button",
                theme.button_info_foreground,
                [
                    theme.button_info,
                    theme.button_info_hover,
                    theme.button_info_active,
                ],
            ),
            (
                "warning button",
                theme.button_warning_foreground,
                [
                    theme.button_warning,
                    theme.button_warning_hover,
                    theme.button_warning_active,
                ],
            ),
        ] {
            for fill in fills {
                for (_, parent) in surfaces(&theme) {
                    assert_readable(name, role, ink, composite(fill.into(), parent));
                }
            }
        }
        for (state, fill) in [
            ("normal", theme.primary),
            ("hover", theme.primary_hover),
            ("active", theme.primary_active),
        ] {
            let ratio = contrast(theme.primary_foreground, fill.into());
            assert!(
                ratio >= 3.,
                "{name}: check mark/{state} contrast {ratio:.4}:1"
            );
        }
    }
}

#[::core::prelude::v1::test]
fn changing_palettes_preserves_shared_roles_within_each_mode() {
    let themes: Vec<_> = projected_themes().collect();
    for (claude, ayu) in [("Claude Light", "Ayu Light"), ("Claude Dark", "Ayu Dark")] {
        let find = |name: &str| {
            themes
                .iter()
                .find(|theme| theme.theme_name() == name)
                .unwrap()
        };
        let claude = find(claude);
        let ayu = find(ayu);
        assert_eq!(claude.mode, ayu.mode);
        assert_ne!(claude.background, ayu.background);
        assert_ne!(claude.group_box, ayu.group_box);
        assert_ne!(claude.button_primary, ayu.button_primary);
        assert_eq!(claude.tokens.border, ayu.tokens.border);
        assert_eq!(claude.tokens.input, ayu.tokens.input);
        assert_eq!(claude.tokens.foreground, ayu.tokens.foreground);
        assert_eq!(claude.tokens.muted_foreground, ayu.tokens.muted_foreground);
        assert_eq!(
            claude.tokens.primary_foreground,
            ayu.tokens.primary_foreground
        );
        assert_eq!(claude.tokens.primary, ayu.tokens.primary);
        assert_eq!(claude.tokens.success, ayu.tokens.success);
        assert_eq!(claude.tokens.danger, ayu.tokens.danger);
        assert_eq!(claude.tokens.warning, ayu.tokens.warning);
        assert_eq!(claude.tokens.link, ayu.tokens.link);
        assert_eq!(claude.tokens.ring, ayu.tokens.ring);
        assert_eq!(
            claude.tokens.switch_thumb_unchecked,
            ayu.tokens.switch_thumb_unchecked
        );
        assert!(std::sync::Arc::ptr_eq(
            &claude.highlight_theme,
            &ayu.highlight_theme
        ));
    }
}
