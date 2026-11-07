// Адаптировано из Zed: crates/ui/src/components/list/list_item.rs
// and crates/ui/src/components/button/button_like.rs (filled vs ghost states).
//! One interaction hierarchy for application rows and native controls.
//! Unfilled rows share a faint selection hover; filled controls retain their
//! own surface and tint it instead of replacing it with a translucent wash.

use std::rc::Rc;

use gpui_kit::component::scroll::ScrollbarMode;
use gpui_kit::component::{Colorize as _, Theme, ThemeConfig};
use gpui_kit::{App, SharedString};

const HOVER_SHARE: f32 = 0.55;
const FILLED_HOVER_INK: f32 = 0.06;
const FILLED_ACTIVE_INK: f32 = 0.12;

/// ThemeConfig clamps list/table active alpha to 0.2. Apply the shared
/// committed selection after loading a config, including System changes.
pub(super) fn install(cx: &mut App) {
    cx.observe_global::<Theme>(enforce).detach();
    cx.defer(enforce);
}

pub(super) fn enforce(cx: &mut App) {
    let theme = Theme::global(cx);
    let selected = theme.sidebar_accent;
    if theme.list_active == selected
        && theme.table_active == selected
        && theme.tokens.list_active.background == selected.into()
        && theme.tokens.table_active.background == selected.into()
        && theme.input == theme.secondary
        && theme.tokens.input == theme.tokens.secondary
        && !theme.focus_ring
        && theme.scrollbar_mode == ScrollbarMode::Always
    {
        return;
    }
    Theme::update(cx, project);
}

fn project(theme: &mut Theme) {
    let selected = theme.sidebar_accent;
    theme.colors.list_active = selected;
    theme.colors.table_active = selected;
    theme.tokens.list_active.background = selected.into();
    theme.tokens.table_active.background = selected.into();
    // Native inputs derive their dark-mode fill from this pigment. Their
    // instance outline stays transparent; the field follows filled controls.
    // Project both token paths so System and custom themes cannot diverge.
    theme.colors.input = theme.secondary;
    theme.tokens.input = theme.tokens.secondary;
    // Native controls keep their existing thin border on keyboard focus,
    // without an outer ring that ancestor clipping would cut into fragments.
    theme.focus_ring = false;
    // Keep the existing quiet idle thumb and brighter hover/drag styles,
    // but do not hide useful scroll position after an idle timeout.
    theme.scrollbar_mode = ScrollbarMode::Always;
}

/// Normalize the selected theme configuration before installing either
/// appearance. Theme::change then projects the same colors onto native
/// component tokens and Base, including later System appearance changes.
pub(super) fn normalize(config: &Rc<ThemeConfig>) -> Rc<ThemeConfig> {
    let mut resolved = Theme::default();
    resolved.apply_config(config);
    let selected = config
        .colors
        .sidebar_accent
        .clone()
        .or_else(|| config.colors.accent.clone())
        .unwrap_or_else(|| resolved.sidebar_accent.to_hex().into());
    let hover: SharedString = resolved.sidebar_accent.opacity(HOVER_SHARE).to_hex().into();
    let mut config = Rc::new(config.as_ref().clone());
    let colors = &mut Rc::make_mut(&mut config).colors;
    colors.input = Some(resolved.secondary.to_hex().into());
    // The default button fill can itself derive from input. Resolve the
    // final input pigment before deriving its states, so a second normalize
    // starts from exactly the same base, even for a sparse custom palette.
    resolved.apply_config(&config);
    let colors = &mut Rc::make_mut(&mut config).colors;
    // Freeze the resolved selected fallback before accent becomes hover;
    // otherwise custom themes without sidebar.accent would inherit hover
    // as their selection and get weaker again on each normalization.
    colors.sidebar_accent = Some(selected.clone());
    // Native menus and ghost buttons read accent, while native list/table
    // rows and our shared selection surface read their named counterparts.
    colors.accent = Some(hover.clone());
    colors.list_hover = Some(hover.clone());
    colors.table_hover = Some(hover);
    colors.list_active = Some(selected.clone());
    colors.table_active = Some(selected);
    let filled_state = |base: gpui_kit::Hsla, ink: f32| -> SharedString {
        base.blend(resolved.foreground.opacity(ink)).to_hex().into()
    };
    colors.button_hover = Some(filled_state(resolved.button, FILLED_HOVER_INK));
    colors.button_active = Some(filled_state(resolved.button, FILLED_ACTIVE_INK));
    colors.secondary_hover = Some(filled_state(resolved.secondary, FILLED_HOVER_INK));
    colors.secondary_active = Some(filled_state(resolved.secondary, FILLED_ACTIVE_INK));
    colors.button_secondary_hover = Some(filled_state(resolved.button_secondary, FILLED_HOVER_INK));
    colors.button_secondary_active =
        Some(filled_state(resolved.button_secondary, FILLED_ACTIVE_INK));
    config
}

#[cfg(test)]
mod tests {
    use super::{HOVER_SHARE, normalize, project};
    use gpui_kit::component::{Theme, ThemeConfig, ThemeSet};
    use std::rc::Rc;

    fn resolve(config: &Rc<ThemeConfig>) -> Theme {
        let mut theme = Theme::default();
        theme.apply_config(config);
        project(&mut theme);
        theme
    }

    #[::core::prelude::v1::test]
    fn projection_restores_selection_after_native_config_alpha_clamp() {
        let set: ThemeSet = serde_json::from_str(super::super::BUNDLED).unwrap();
        for config in set.themes {
            let config = normalize(&Rc::new(config));
            let mut theme = Theme::default();
            theme.apply_config(&config);
            assert!(theme.list_active.a <= 0.2);
            project(&mut theme);
            assert_eq!(theme.list_active, theme.sidebar_accent);
            assert_eq!(theme.table_active, theme.sidebar_accent);
            assert_eq!(
                theme.tokens.list_active.background,
                theme.sidebar_accent.into()
            );
            assert_eq!(
                theme.tokens.table_active.background,
                theme.sidebar_accent.into()
            );
        }
    }

    #[::core::prelude::v1::test]
    fn projection_disables_native_outer_focus_rings_after_theme_changes() {
        let mut theme = Theme::default();
        theme.focus_ring = true;
        project(&mut theme);
        assert!(!theme.focus_ring);
        let colors = theme.colors.clone();
        // A later System/custom theme can restore the native default. The
        // projection must still run when the color tokens already match.
        theme.focus_ring = true;
        project(&mut theme);
        assert!(!theme.focus_ring);
        assert_eq!(theme.colors, colors);
    }

    #[::core::prelude::v1::test]
    fn projection_keeps_overflow_scrollbars_visible_after_system_changes() {
        use gpui_kit::component::scroll::ScrollbarMode;
        let mut theme = Theme::default();
        let colors = theme.colors.clone();
        for mode in [ScrollbarMode::Scrolling, ScrollbarMode::Hover] {
            theme.scrollbar_mode = mode;
            project(&mut theme);
            assert_eq!(theme.scrollbar_mode, ScrollbarMode::Always);
        }
        // Visibility policy reuses theme colors for idle/hover/drag.
        assert_eq!(theme.colors.scrollbar_thumb, colors.scrollbar_thumb);
        assert_eq!(
            theme.colors.scrollbar_thumb_hover,
            colors.scrollbar_thumb_hover
        );
    }

    #[::core::prelude::v1::test]
    fn bundled_appearances_keep_hover_weaker_than_committed_selection() {
        let set: ThemeSet = serde_json::from_str(super::super::BUNDLED).unwrap();
        for config in set.themes {
            let config = Rc::new(config);
            let original = resolve(&config);
            let normalized = normalize(&config);
            let theme = resolve(&normalized);
            assert_eq!(theme.sidebar_accent, original.sidebar_accent);
            assert!(theme.list_hover.a <= theme.sidebar_accent.a * HOVER_SHARE + 0.005);
            assert!(theme.list_hover.a < theme.sidebar_accent.a);
            assert_eq!(theme.list_hover, theme.accent);
            assert_eq!(theme.list_hover, theme.table_hover);
            assert_eq!(theme.list_active, theme.sidebar_accent);
            assert_eq!(theme.table_active, theme.sidebar_accent);
            for (base, hovered, active) in [
                (theme.button, theme.button_hover, theme.button_active),
                (
                    theme.secondary,
                    theme.secondary_hover,
                    theme.secondary_active,
                ),
                (
                    theme.button_secondary,
                    theme.button_secondary_hover,
                    theme.button_secondary_active,
                ),
            ] {
                assert_eq!(hovered.a, 1.);
                assert_eq!(active.a, 1.);
                assert_ne!(hovered, base);
                assert_ne!(active, base);
                assert_ne!(hovered, active);
                assert_ne!(hovered, theme.list_hover);
            }
            assert_eq!(theme.button, original.button);
            assert_eq!(theme.secondary, original.secondary);
            assert_eq!(theme.button_secondary, original.button_secondary);
            // Color projection must reach native token consumers too.
            assert_eq!(theme.tokens.accent.background, theme.accent.into());
            assert_eq!(theme.tokens.list_hover.background, theme.list_hover.into());
            assert_eq!(theme.primary, original.primary);
            assert_eq!(theme.input, theme.secondary);
            assert_eq!(theme.tokens.input, theme.tokens.secondary);
            assert_eq!(theme.danger, original.danger);
            assert_eq!(resolve(&normalize(&normalized)).colors, theme.colors);
        }
    }

    #[::core::prelude::v1::test]
    fn custom_palette_derives_hover_from_its_own_selected_color() {
        let config: ThemeConfig = serde_json::from_value(serde_json::json!({
            "name":"Synthetic custom theme", "mode":"dark", "colors":{
                "sidebar.accent.background":"blue-600/80",
                "accent.background":"red-500",
                "list.hover.background":"green-500",
                "border":"blue-500",
                "input.border":"red-500"
            }
        }))
        .unwrap();
        let original = resolve(&Rc::new(config.clone()));
        let normalized = normalize(&Rc::new(config));
        let theme = resolve(&normalized);
        assert_eq!(theme.sidebar_accent, original.sidebar_accent);
        assert_eq!(theme.input, theme.secondary);
        assert_eq!(theme.tokens.input, theme.tokens.secondary);
        assert!(theme.accent.a < theme.sidebar_accent.a);
        assert!((theme.accent.h - theme.sidebar_accent.h).abs() < 0.01);
        assert!((theme.accent.s - theme.sidebar_accent.s).abs() < 0.01);
        // The config's hex encoding rounds alpha to an 8-bit channel.
        assert!((theme.button_hover.a - theme.button.a).abs() <= 1. / 255.);
        assert!((theme.button_active.a - theme.button.a).abs() <= 1. / 255.);
        assert_ne!(theme.button_hover, theme.button);
        assert_ne!(theme.button_active, theme.button);
        assert_ne!(theme.button_hover, theme.button_active);
        assert_eq!(resolve(&normalize(&normalized)).colors, theme.colors);
    }

    #[::core::prelude::v1::test]
    fn a_custom_theme_without_sidebar_color_keeps_its_original_selected_fallback() {
        let config: ThemeConfig = serde_json::from_value(serde_json::json!({
            "name":"Synthetic fallback theme", "mode":"light", "colors":{
                "accent.background":"blue-200"
            }
        }))
        .unwrap();
        let original = resolve(&Rc::new(config.clone()));
        let normalized = normalize(&Rc::new(config));
        let theme = resolve(&normalized);
        assert_eq!(theme.sidebar_accent, original.sidebar_accent);
        assert!(theme.accent.a < theme.sidebar_accent.a);
        assert_eq!(resolve(&normalize(&normalized)).colors, theme.colors);
    }
}
