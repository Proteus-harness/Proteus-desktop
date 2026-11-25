use std::{rc::Rc, sync::Arc};

use gpui_kit::Hsla;
use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::{Theme, ThemeColor, ThemeConfig, ThemeToken, ThemeTokens};

pub(super) struct SurfacePalette {
    dark: AppearanceColors,
    light: AppearanceColors,
}

struct AppearanceColors {
    colors: ThemeColor,
    tokens: ThemeTokens,
    highlight: Arc<HighlightTheme>,
}

impl AppearanceColors {
    fn new(config: &Rc<ThemeConfig>) -> Self {
        let mut theme = Theme::default();
        theme.apply_config(config);
        Self {
            colors: theme.colors,
            tokens: theme.tokens,
            highlight: theme.highlight_theme,
        }
    }
}

impl SurfacePalette {
    pub(super) fn new(dark: &Rc<ThemeConfig>, light: &Rc<ThemeConfig>) -> Self {
        Self {
            dark: AppearanceColors::new(dark),
            light: AppearanceColors::new(light),
        }
    }

    pub(super) fn apply(&self, theme: &mut Theme) {
        let base = if theme.is_dark() {
            &self.dark
        } else {
            &self.light
        };
        let source = theme.tokens;
        theme.colors = base.colors;
        theme.tokens = base.tokens;
        theme.highlight_theme = Arc::clone(&base.highlight);

        // These roles are surface fills. The schema also calls progress,
        // thumbs and state ink (like `danger` and `primary`) backgrounds.
        macro_rules! apply_fills {
            ($($field:ident),+ $(,)?) => {
                $(theme.colors.$field = source.$field.color;
                  theme.tokens.$field = source.$field;)+
            };
        }
        apply_fills!(
            accent,
            accordion,
            background,
            button,
            button_active,
            button_hover,
            button_secondary,
            button_secondary_active,
            button_secondary_hover,
            description_list_label,
            group_box,
            input_background,
            list,
            list_active,
            list_even,
            list_head,
            list_hover,
            muted,
            overlay,
            panel_header_hover,
            popover,
            scrollbar,
            secondary,
            secondary_active,
            secondary_hover,
            sidebar,
            sidebar_accent,
            skeleton,
            status_bar,
            tab,
            tab_active,
            tab_bar,
            tab_bar_segmented,
            table,
            table_active,
            table_even,
            table_foot,
            table_head,
            table_hover,
            title_bar,
        );

        // Keep one ink across the solid button's three states. Saturated
        // themes with a light fill sometimes supply their own dark text.
        if [
            source.button_primary,
            source.button_primary_hover,
            source.button_primary_active,
        ]
        .into_iter()
        .all(|fill| readable_solid(theme.button_primary_foreground, fill))
        {
            apply_fills!(button_primary, button_primary_hover, button_primary_active);
        }
    }
}

fn readable_solid(ink: Hsla, fill: ThemeToken) -> bool {
    let Some(fill) = fill.background.as_solid().filter(|color| color.a == 1.) else {
        return false;
    };
    fn luminance(color: Hsla) -> f64 {
        let rgb = color.to_rgb();
        let linear = |v: f32| {
            let v = f64::from(v);
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
    }
    let a = luminance(fill.blend(ink));
    let b = luminance(fill);
    (a.max(b) + 0.05) / (a.min(b) + 0.05) >= 4.5
}
