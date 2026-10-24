// Адаптировано из Zed: crates/ui/src/traits/styled_ext.rs (surface borders).
//! The neutral outline of controls and containers. Backgrounds and state
//! colours belong to the caller; width, default radius and depth are shared.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, ParentElement, Styled, div, px};

use super::super::prefs;

pub(crate) trait Frame: Styled + Sized {
    fn frame(self, cx: &App) -> Self {
        self.border_1()
            .border_color(cx.theme().border)
            .rounded(cx.theme().radius)
            .shadow_none()
    }
}

impl<T: Styled> Frame for T {}

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
