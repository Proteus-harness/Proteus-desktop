// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_settings_item_layout)
//! Shared geometry for native and extension settings. Ordinary parameters
//! stay on one line and shrink to the available width. Only explicit multiline
//! editors belong below their title; no intrinsic-width wrap moves a selector.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::ui::prefs;

pub(crate) fn settings_row(title: impl IntoElement, control: impl IntoElement, below: bool) -> Div {
    let title = h_flex()
        .min_w_0()
        .gap_2()
        .items_center()
        .text_sm()
        .child(title);
    if below {
        return v_flex()
            .min_w_0()
            .px_4()
            .py_2p5()
            .gap_2()
            .child(title)
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .max_w(prefs::scaled(560.))
                    .child(control),
            );
    }
    h_flex()
        .min_w_0()
        .px_4()
        .py_2p5()
        .gap_2()
        .items_center()
        .child(title.flex_1())
        .child(div().min_w_0().flex_shrink_1().max_w_full().child(control))
}
