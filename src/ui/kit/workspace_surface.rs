// Адаптировано из Zed: crates/ui/src/traits/styled_ext.rs (shared surface styling).
//! Upper workspace corners join their surrounding chrome without changing
//! layout or hit targets. Applied last, the cover also masks child fills.

use gpui_kit::base::RoundedFrameCover;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{Hsla, ParentElement, Pixels, Styled, div, px};

pub(crate) trait WorkspaceSurface: Styled + ParentElement + FluentBuilder + Sized {
    /// Use the shared surface radius and the actual surrounding backdrop.
    /// GPUI's rectangular child mask alone cannot protect rounded corners.
    fn workspace_surface(self, radius: Pixels, backdrop: Hsla) -> Self {
        self.relative()
            .rounded_t(radius)
            .when(radius > px(0.), |surface| {
                surface.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(radius)
                        .overflow_hidden()
                        .child(
                            div()
                                .relative()
                                .w_full()
                                .h(radius * 2.)
                                // RoundedFrameCover follows a flow child and shifts
                                // its static position upward by its own height.
                                .child(div().w_full().h_full())
                                .child(RoundedFrameCover::uniform(radius).backdrop(backdrop)),
                        ),
                )
            })
    }
}

impl<T: Styled + ParentElement + FluentBuilder> WorkspaceSurface for T {}
