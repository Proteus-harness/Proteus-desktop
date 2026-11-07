// Адаптировано из Zed: crates/ui/src/traits/styled_ext.rs (surface borders).
// Filled states follow crates/ui/src/components/button/button_like.rs.
//! Flat controls and containers share their radius and a transparent layout
//! edge. Callers can tint that edge for a semantic state without resizing it.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Bounds, Hsla, ParentElement, Pixels, Styled, Window};
use std::{cell::Cell, rc::Rc};

pub(crate) trait Frame: Styled + Sized {
    fn frame(self, cx: &App) -> Self {
        self.border_1()
            .border_color(cx.theme().transparent)
            .rounded(cx.theme().radius)
            .shadow_none()
    }

    /// Panels and cards share the composer's corner size. Compact controls
    /// keep `frame` so a large surface radius cannot turn them into capsules.
    fn surface_frame(self, cx: &App) -> Self {
        self.frame(cx).rounded(cx.theme().radius_lg)
    }

    /// A bounded disclosure owns one visible panel around its header and body,
    /// including when that panel is nested inside a group-box card.
    fn disclosure_frame(self, cx: &App) -> Self {
        self.surface_frame(cx)
            .border_0()
            .bg(cx.theme().group_box)
            .text_color(cx.theme().foreground)
            .overflow_hidden()
    }
}

impl<T: Styled> Frame for T {}

/// Layer the theme's authored interaction colors over the card's own fill.
/// A translucent highlight must not replace the surface beneath it.
pub(crate) fn surface_hover(cx: &App) -> Hsla {
    cx.theme().group_box.blend(cx.theme().list_hover)
}

pub(crate) fn surface_selected(cx: &App) -> Hsla {
    cx.theme().group_box.blend(cx.theme().list_active)
}

/// Record the first child's outer layout box from its containing Div.
/// ElementExt::on_prepaint measures an added canvas inside the frame instead.
pub(crate) fn record_first_child(
    bounds: Rc<Cell<Bounds<Pixels>>>,
) -> impl Fn(Vec<Bounds<Pixels>>, &mut Window, &mut App) {
    move |children, _, _| {
        if let Some(rect) = children.first() {
            bounds.set(*rect);
        }
    }
}

/// Preserve the existing hairline of layout space between settings rows,
/// without painting a decorative separator over their containing surface.
pub(crate) trait RowRule: Styled + ParentElement + Sized {
    fn row_rule(self, cx: &App) -> Self {
        self.relative()
            .border_t_1()
            .border_color(cx.theme().transparent)
    }
}

impl<T: Styled + ParentElement> RowRule for T {}
