// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_settings_item_layout)
//! Shared settings geometry. Short controls align to the right, long inputs
//! wrap below their caption if the form is narrow, and multiline editors
//! always go below.

use gpui_kit::component::h_flex;
use gpui_kit::*;

use crate::ui::prefs;

pub(super) fn settings_row_height() -> Rems {
    // A card's transparent separator supplies the extra pixel between rows.
    prefs::scaled(35.)
}

pub(super) fn settings_row_padding() -> Rems {
    prefs::scaled(4.)
}

#[derive(IntoElement)]
pub(crate) struct SettingsRow {
    title: AnyElement,
    control: AnyElement,
    below: bool,
    fill_control: bool,
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
        fill_control: false,
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

    pub(crate) fn fill_control(mut self) -> Self {
        self.fill_control = true;
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
            .line_height(super::compact_row_line_height())
            .child(self.title);
        let row = div().flex().min_w_0().px_4();
        let mut row = if self.below {
            row.flex_col()
                .py_1p5()
                .gap_1p5()
                .child(title)
                .child(div().w_full().min_w_0().child(self.control))
        } else {
            let row = row
                .flex_row()
                .min_h(settings_row_height())
                .py(settings_row_padding())
                .gap_x_2()
                .items_center();
            if self.fill_control {
                row.w_full()
                    .flex_wrap()
                    .gap_y_1p5()
                    // Keep the full caption width available on a wrapped line.
                    .child(title.flex_initial().max_w_full())
                    .child(
                        div()
                            .flex_1()
                            .flex_basis(prefs::scaled(self.minimum_control_width.max(192.)))
                            .min_w_0()
                            .child(self.control),
                    )
            } else {
                row.child(title.flex_initial()).child(
                    h_flex()
                        .flex_1()
                        .min_w(prefs::scaled(self.minimum_control_width))
                        .justify_end()
                        .child(div().min_w_0().max_w_full().child(self.control)),
                )
            }
        };
        row.style().refine(&self.style);
        row
    }
}
