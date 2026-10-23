// Адаптировано из Zed: crates/ui/src/components/toggle.rs, crates/ui/src/components/diff_stat.rs, crates/ui/src/components/chip.rs
//! Small pieces of rows: a three-state checkbox, a diff stat and counter
//! and label chips, sized like their Zed counterparts.

use crate::declarative::tree::Check;
use crate::ui::prefs;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// A 16 px box in a 20 px hit area: a check when on, a dash when mixed.
pub(super) fn checkbox(id: impl Into<ElementId>, state: Check, cx: &App) -> Stateful<Div> {
    let id = id.into();
    let theme = cx.theme();
    let group = SharedString::from(format!("check-{id}"));
    let (border, mark) = (theme.border, theme.primary);
    let hover = border.opacity(0.7);
    let icon = match state {
        Check::On => Some(Lucide::Check),
        Check::Mixed => Some(Lucide::Minus),
        Check::Off => None,
    };
    h_flex()
        .id(id)
        .group(group.clone())
        .flex_none()
        .size(prefs::scaled(20.))
        .justify_center()
        .cursor_pointer()
        .child(
            h_flex()
                .flex_none()
                .justify_center()
                .size(prefs::scaled(16.))
                .rounded(px(3.))
                .border_1()
                .border_color(border)
                .bg(theme.secondary)
                .when(state != Check::Off, |check| {
                    check.border_color(mark.opacity(0.6))
                })
                .group_hover(group, move |check| check.border_color(hover))
                .children(icon.map(|icon| Icon::new(icon).xsmall().text_color(mark))),
        )
}

/// `+added −deleted` in the success and danger colors.
pub(super) fn diff_stat(added: u64, deleted: u64, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .flex_none()
        .gap_1()
        .text_xs()
        .child(
            div()
                .text_color(theme.success)
                .child(format!("+\u{2009}{added}")),
        )
        .child(
            div()
                .text_color(theme.danger)
                .child(format!("\u{2012}\u{2009}{deleted}")),
        )
}

/// A muted number in a thin frame.
pub(super) fn chip(count: usize, cx: &App) -> Div {
    frame(cx).flex_none().child(count.to_string())
}

/// A muted label in the same frame, such as a tag; like Zed's truncating
/// `Chip` it gives way when the row is short of room.
pub(super) fn label_chip(label: impl Into<SharedString>, cx: &App) -> Div {
    frame(cx)
        .min_w_0()
        .overflow_hidden()
        .child(crate::ui::fade::faded(label.into()))
}

fn frame(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1()
        .rounded(px(3.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_xs()
        .line_height(prefs::scaled(14.))
        .text_color(theme.muted_foreground)
}
