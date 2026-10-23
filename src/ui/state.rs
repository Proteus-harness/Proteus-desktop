//! Shared view states for client pages and process widgets.
use gpui_kit::component::*;
use gpui_kit::*;

pub(super) fn loading(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    cx: &App,
) -> AnyElement {
    super::motion::late_in(
        h_flex()
            .w_full()
            .justify_center()
            .items_center()
            .gap_2()
            .py_3()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(super::motion::Spinner::new().small())
            .child(text.into()),
        id,
    )
    .into_any_element()
}
