// Адаптировано из Zed: crates/extensions_ui/src/components/extension_card.rs.
//! Shared heading for module, extension and catalog cards: an icon beside a title
//! and a separate tag line. Row padding and trailing actions belong to the
//! enclosing item_row, so both pages keep the same geometry.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::prefs;

pub(in crate::ui::settings) fn package_heading(
    icon: Icon,
    title: impl IntoElement,
    tags: impl IntoIterator<Item = AnyElement>,
) -> Div {
    let tags = tags.into_iter().collect::<Vec<_>>();
    h_flex()
        .flex_1()
        .min_w_0()
        .gap_3()
        .items_center()
        .child(icon.size(prefs::scaled(24.)).flex_none())
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(h_flex().min_w_0().text_sm().font_medium().child(title))
                .when(!tags.is_empty(), |text| {
                    text.child(h_flex().min_w_0().flex_wrap().gap_1().children(tags))
                }),
        )
}
