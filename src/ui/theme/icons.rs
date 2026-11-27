use gpui_kit::component::{IconTone, Theme};
use gpui_kit::{Hsla, Rgba};

pub(in crate::ui) fn color(tone: IconTone, theme: &Theme) -> Hsla {
    match tone {
        IconTone::Normal => theme.foreground,
        IconTone::Muted => theme.muted_foreground,
        IconTone::Selected => theme.sidebar_accent_foreground,
        IconTone::Accent => theme.link,
        IconTone::Success => success_text(theme),
        IconTone::Warning => warning_text(theme),
        IconTone::Danger => danger_text(theme),
        IconTone::Custom(color) => color,
    }
}

pub(in crate::ui) fn danger_text(theme: &Theme) -> Hsla {
    status_text(theme.danger, theme)
}

pub(in crate::ui) fn success_text(theme: &Theme) -> Hsla {
    status_text(theme.success, theme)
}

pub(in crate::ui) fn warning_text(theme: &Theme) -> Hsla {
    status_text(theme.warning, theme)
}

fn status_text(color: Hsla, theme: &Theme) -> Hsla {
    let surfaces = [
        theme.background,
        theme.group_box,
        theme.sidebar,
        theme.popover,
    ];
    let readable = |ink: Hsla| {
        surfaces.iter().all(|surface| {
            let foreground = luminance(surface.blend(ink));
            let background = luminance(*surface);
            (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05) >= 4.5
        })
    };
    if readable(color) {
        return color;
    }

    // Move toward the palette's readable text color while keeping the status hue.
    let mut low = 0.;
    let mut high = 1.;
    for _ in 0..8 {
        let midpoint = (low + high) / 2.;
        if readable(color.blend(theme.foreground.opacity(midpoint))) {
            high = midpoint;
        } else {
            low = midpoint;
        }
    }
    color.blend(theme.foreground.opacity(high))
}

fn luminance(color: Hsla) -> f32 {
    let Rgba { r, g, b, .. } = color.into();
    let linear = |value: f32| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}
