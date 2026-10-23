//! Selection shared by navigation and data rows: an inset rounded surface,
//! independent of table rules and the keyboard-only focus outline.
use super::prefs;
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
