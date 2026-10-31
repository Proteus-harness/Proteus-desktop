// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_settings_item_layout)
//! Shared settings geometry. The caption keeps its intrinsic width while
//! the control gives up spare room. Only after the control reaches its usable
//! minimum does the caption shrink. Multiline editors explicitly go below.

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

use crate::ui::prefs;

#[derive(IntoElement)]
pub(crate) struct SettingsRow {
    title: AnyElement,
    control: AnyElement,
    below: bool,
    minimum_control_width: f32,
    style: StyleRefinement,
}

pub(crate) fn settings_row(
    title: impl IntoElement,
    control: impl IntoElement,
    below: bool,
) -> SettingsRow {
    SettingsRow {
        title: title.into_any_element(),
        control: control.into_any_element(),
        below,
        // The standard switch is 36px wide. Wider controls opt into a useful
        // minimum; their preferred width still belongs to the control itself.
        minimum_control_width: 36.,
        style: StyleRefinement::default(),
    }
}

impl SettingsRow {
    pub(crate) fn minimum_control_width(mut self, width: f32) -> Self {
        self.minimum_control_width = width;
        self
    }
}

impl Styled for SettingsRow {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let title = h_flex()
            .min_w_0()
            .gap_2()
            .items_center()
            .text_sm()
            .child(self.title);
        let mut row = if self.below {
            v_flex()
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
                        .child(self.control),
                )
        } else {
            h_flex()
                .min_w_0()
                .px_4()
                .py_2p5()
                .gap_2()
                .items_center()
                .child(title.flex_initial())
                .child(
                    h_flex()
                        .flex_1()
                        .min_w(prefs::scaled(self.minimum_control_width))
                        .justify_end()
                        .child(div().min_w_0().max_w_full().child(self.control)),
                )
        };
        row.style().refine(&self.style);
        row
    }
}
