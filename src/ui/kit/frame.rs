// Адаптировано из Zed: crates/ui/src/traits/styled_ext.rs (surface borders).
//! The neutral outline of controls and containers. Backgrounds and state
//! colours belong to the caller; width, default radius and depth are shared.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, Bounds, ParentElement, Pixels, Styled, Window, div, px};
use std::{cell::Cell, rc::Rc};

use super::super::prefs;

pub(crate) trait Frame: Styled + Sized {
    fn frame(self, cx: &App) -> Self {
        self.border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .shadow_none()
    }

    /// Panels and cards share the composer's corner size. Compact controls
    /// keep `frame` so a large surface radius cannot turn them into capsules.
    fn surface_frame(self, cx: &App) -> Self {
        self.frame(cx).rounded(cx.theme().radius_lg)
    }
}

impl<T: Styled> Frame for T {}

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

/// A quiet rule between settings rows. The transparent border reserves the
/// same hairline of space without changing the row's padding or hit target.
/// The painted line follows the content inset, including inside nested groups.
pub(crate) trait RowRule: Styled + ParentElement + Sized {
    fn row_rule(self, cx: &App) -> Self {
        self.relative()
            .border_t_1()
            .border_color(cx.theme().transparent)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left(prefs::scaled(16.))
                    .right(prefs::scaled(16.))
                    .h(px(1.))
                    .bg(cx.theme().border.opacity(0.6)),
            )
    }
}

impl<T: Styled + ParentElement> RowRule for T {}
