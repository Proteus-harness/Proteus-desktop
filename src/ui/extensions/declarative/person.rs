//! Local initials shared by person columns and profile headers.
use crate::ui::prefs;
use gpui_kit::component::*;
use gpui_kit::*;

pub(super) fn avatar(name: &str, size: f32, cx: &App) -> Div {
    let initials: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    h_flex()
        .flex_none()
        .size(prefs::scaled(size))
        .justify_center()
        .rounded_full()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
        .text_size(prefs::scaled(size * 0.45))
        .text_color(cx.theme().muted_foreground)
        .child(if initials.is_empty() {
            "?".to_owned()
        } else {
            initials
        })
}
