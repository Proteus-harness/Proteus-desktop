// Адаптировано из Zed: crates/ui/src/components/list/list_item.rs
//! Interaction policies applied independently of surface colors.

use gpui_kit::App;
use gpui_kit::component::Theme;
use gpui_kit::component::scroll::ScrollbarMode;

pub(super) fn install(cx: &mut App) {
    cx.defer(enforce);
}

pub(super) fn enforce(cx: &mut App) {
    let theme = Theme::global(cx);
    if !theme.focus_ring && theme.scrollbar_mode == ScrollbarMode::Always {
        return;
    }
    Theme::update(cx, project);
}

fn project(theme: &mut Theme) {
    // Keep the native keyboard marker inside the control rather than adding
    // an outer ring that clipped containers would cut into fragments.
    theme.focus_ring = false;
    // Scrollbar visibility follows the application policy in both modes.
    theme.scrollbar_mode = ScrollbarMode::Always;
}

#[cfg(test)]
mod tests {
    use super::project;
    use gpui_kit::component::scroll::ScrollbarMode;
    use gpui_kit::component::{Colorize, Theme, ThemeConfig, ThemeSet};
    use gpui_kit::{Hsla, transparent_black};
    use std::rc::Rc;

    fn assert_palette_preserved(mut theme: Theme) {
        let colors = theme.colors.clone();
        let tokens = theme.tokens;
        for _ in 0..2 {
            project(&mut theme);
            assert_eq!(theme.colors, colors);
            assert_eq!(theme.tokens, tokens);
            assert!(!theme.focus_ring);
            assert_eq!(theme.scrollbar_mode, ScrollbarMode::Always);
        }
    }

    #[::core::prelude::v1::test]
    fn bundled_palettes_keep_authored_colors_and_native_fallbacks() {
        let sources = std::iter::once(super::super::BUNDLED)
            .chain(super::super::palettes::BUNDLED.iter().copied());
        for source in sources {
            let set: ThemeSet = serde_json::from_str(source).unwrap();
            for config in set.themes {
                let mut theme = Theme::default();
                theme.apply_config(&Rc::new(config));
                assert_palette_preserved(theme);
            }
        }
    }

    #[::core::prelude::v1::test]
    fn custom_palettes_keep_explicit_input_selection_and_button_states() {
        for mode in ["dark", "light"] {
            let config: ThemeConfig = serde_json::from_value(serde_json::json!({
                "name":"Synthetic authored palette", "mode":mode, "colors":{
                    "accent.background":"red-500",
                    "sidebar.accent.background":"blue-600/80",
                    "list.active.background":"green-500/15",
                    "list.hover.background":"green-500/10",
                    "panel.header.hover.background":"#2468ac20",
                    "table.active.background":"amber-500/15",
                    "table.hover.background":"amber-500/10",
                    "border":"blue-500",
                    "input.border":"red-500",
                    "input.background":"#234567c0",
                    "secondary.background":"gray-700",
                    "secondary.hover.background":"gray-600",
                    "secondary.active.background":"gray-500",
                    "button.background":"indigo-700",
                    "button.hover.background":"indigo-600",
                    "button.active.background":"indigo-500",
                    "button.secondary.hover.background":"purple-600",
                    "button.secondary.active.background":"purple-500"
                }
            }))
            .unwrap();
            let mut theme = Theme::default();
            theme.apply_config(&Rc::new(config));
            assert_eq!(
                theme.input_background(),
                Hsla::parse_hex("#234567c0").unwrap()
            );
            assert_eq!(
                theme.panel_header_hover,
                Hsla::parse_hex("#2468ac20").unwrap()
            );
            assert_eq!(
                theme.tokens.input_background.color,
                theme.input_background()
            );
            assert_palette_preserved(theme);
        }
    }

    #[::core::prelude::v1::test]
    fn sparse_palettes_keep_the_frameworks_derived_colors() {
        for mode in ["dark", "light"] {
            let config: ThemeConfig = serde_json::from_value(serde_json::json!({
                "name":"Synthetic sparse palette", "mode":mode, "colors":{
                    "accent.background":"blue-200",
                    "background":"#142536",
                    "input.border":"#6789ab"
                }
            }))
            .unwrap();
            let mut theme = Theme::default();
            // Applying a custom theme after an opaque builtin must not leak
            // that builtin's input role into the custom palette.
            let builtins: ThemeSet = serde_json::from_str(super::super::BUNDLED).unwrap();
            let builtin = builtins
                .themes
                .into_iter()
                .find(|theme| theme.mode.is_dark() == (mode == "dark"))
                .unwrap();
            theme.apply_config(&Rc::new(builtin));
            theme.apply_config(&Rc::new(config));
            let native_fill = if mode == "dark" {
                theme.input.mix_oklab(transparent_black(), 0.3)
            } else {
                theme.background
            };
            assert_eq!(theme.input_background(), native_fill);
            assert_eq!(theme.tokens.input_background.color, native_fill);
            assert_eq!(theme.panel_header_hover, theme.list_hover);
            assert_palette_preserved(theme);
        }
    }

    #[::core::prelude::v1::test]
    fn projection_disables_native_outer_focus_rings_after_theme_changes() {
        let mut theme = Theme::default();
        theme.focus_ring = true;
        project(&mut theme);
        assert!(!theme.focus_ring);
        let colors = theme.colors.clone();
        let tokens = theme.tokens;
        // A later System/custom theme can restore the native default even
        // when its palette matches the currently installed one.
        theme.focus_ring = true;
        project(&mut theme);
        assert!(!theme.focus_ring);
        assert_eq!(theme.colors, colors);
        assert_eq!(theme.tokens, tokens);
    }

    #[::core::prelude::v1::test]
    fn projection_keeps_overflow_scrollbars_visible_after_system_changes() {
        let mut theme = Theme::default();
        let colors = theme.colors.clone();
        let tokens = theme.tokens;
        for mode in [ScrollbarMode::Scrolling, ScrollbarMode::Hover] {
            theme.scrollbar_mode = mode;
            project(&mut theme);
            assert_eq!(theme.scrollbar_mode, ScrollbarMode::Always);
        }
        assert_eq!(theme.colors, colors);
        assert_eq!(theme.tokens, tokens);
    }
}
