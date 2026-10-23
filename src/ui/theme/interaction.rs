// Адаптировано из Zed: crates/ui/src/components/list/list_item.rs
//! One interaction hierarchy for application rows and native controls.
//! Hover is a faint version of the theme's committed selection; cursor and
//! focus cannot acquire the same visual weight as a selected value.

use std::rc::Rc;

use gpui_kit::component::{Colorize as _, Theme, ThemeConfig};
use gpui_kit::{App, SharedString};

const HOVER_SHARE: f32 = 0.55;

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
    let mut config = config.as_ref().clone();
    let colors = &mut config.colors;
    // Freeze the resolved selected fallback before accent becomes hover;
    // otherwise custom themes without sidebar.accent would inherit hover
    // as their selection and get weaker again on each normalization.
    colors.sidebar_accent = Some(selected.clone());
    // Native menus and ghost buttons read accent, while native list/table
    // rows and our shared selection surface read their named counterparts.
    colors.accent = Some(hover.clone());
    colors.list_hover = Some(hover.clone());
    colors.table_hover = Some(hover.clone());
    colors.button_hover = Some(hover.clone());
    colors.secondary_hover = Some(hover.clone());
    colors.button_secondary_hover = Some(hover);
    colors.list_active = Some(selected.clone());
    colors.table_active = Some(selected.clone());
    colors.button_active = Some(selected.clone());
    colors.secondary_active = Some(selected.clone());
    colors.button_secondary_active = Some(selected);
    Rc::new(config)
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
            assert_eq!(theme.list_hover, theme.button_secondary_hover);
            assert_eq!(theme.list_active, theme.sidebar_accent);
            assert_eq!(theme.table_active, theme.sidebar_accent);
            assert_eq!(theme.secondary_active, theme.sidebar_accent);
            assert_eq!(theme.button_active, theme.sidebar_accent);
            // Color projection must reach native token consumers too.
            assert_eq!(theme.tokens.accent.background, theme.accent.into());
            assert_eq!(theme.tokens.list_hover.background, theme.list_hover.into());
            assert_eq!(theme.primary, original.primary);
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
                "list.hover.background":"green-500"
            }
        }))
        .unwrap();
        let original = resolve(&Rc::new(config.clone()));
        let theme = resolve(&normalize(&Rc::new(config)));
        assert_eq!(theme.sidebar_accent, original.sidebar_accent);
        assert!(theme.accent.a < theme.sidebar_accent.a);
        assert!((theme.accent.h - theme.sidebar_accent.h).abs() < 0.01);
        assert!((theme.accent.s - theme.sidebar_accent.s).abs() < 0.01);
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
