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
            .width()
            .ceil();
        h_flex()
            .id("menu-option")
            .aria_label(label.clone())
            .aria_selected(selected)
            .relative()
            .w(natural_width)
            .min_w_0()
            .flex_grow(1.)
            .text_sm()
            .min_h(prefs::scaled(26.))
            .when(selected, |row| {
                row.text_color(theme.sidebar_accent_foreground)
            })
            // Keep native menu padding in the layout. Only the selected
            // paint extends into it; negative margins on the caption used
            // to reduce its measured room and fade fitting final letters.
            .when(selected, |row| {
                row.child(
                    div()
                        .absolute()
                        .left(px(-8.))
                        .right(px(-8.))
                        .top_0()
                        .bottom_0()
                        .rounded(theme.radius)
                        .bg(theme.sidebar_accent),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(super::fade::caption("menu-option-caption", label.clone())),
            )
    })
}
