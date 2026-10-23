//! Selection shared by navigation and data rows: an inset rounded surface,
//! independent of table rules and the keyboard-only focus outline.
use super::prefs;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(crate) fn background(
    selected: bool,
    cursor: bool,
    group: &'static str,
    inset: bool,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let hover = theme.list_hover;
    div()
        .absolute()
        .inset_0()
        .when(inset, |s| {
            s.left(prefs::scaled(3.))
                .right(prefs::scaled(3.))
                .top(prefs::scaled(2.))
                .bottom(prefs::scaled(2.))
        })
        .rounded(theme.radius)
        .when(selected, |s| s.bg(theme.sidebar_accent))
        .when(!selected && cursor, |s| s.bg(hover))
        .when(!selected, |s| s.group_hover(group, move |s| s.bg(hover)))
}

pub(crate) fn focus(cx: &App) -> Div {
    div()
        .absolute()
        .left(prefs::scaled(3.))
        .right(prefs::scaled(3.))
        .top(prefs::scaled(2.))
        .bottom(prefs::scaled(2.))
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().ring)
}

/// A committed single-choice value, independent of the menu's hover/keyboard
/// cursor. The menu retains its native navigation and intrinsic text width;
/// the selected value uses the same inset surface as rows and model choices.
pub(crate) fn menu_option(label: impl Into<SharedString>, selected: bool) -> PopupMenuItem {
    let label = label.into();
    PopupMenuItem::element(move |window, cx| {
        let theme = cx.theme();
        let font = prefs::scaled(14.).to_pixels(window.rem_size());
        let style = window.text_style();
        let natural_width = window
            .text_system()
            .shape_line(label.clone(), font, &[style.to_run(label.len())], None)
            .width();
        h_flex()
            .id("menu-option")
            .aria_label(label.clone())
            .aria_selected(selected)
            .relative()
            .w(natural_width + prefs::scaled(8.).to_pixels(window.rem_size()))
            .min_w_0()
            .flex_1()
            .text_sm()
            // The native MenuItemElement has 8px side padding. Extending
            // the child by 4px leaves the selected pill inset by 4px while
            // the caption stays aligned with ordinary menu entries.
            .mx_neg_1()
            .px_1()
            .min_h(prefs::scaled(26.))
            .rounded(theme.radius)
            .when(selected, |row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(super::fade::caption("menu-option-caption", label.clone())),
            )
    })
}
