//! Page-owned multiline fields share the page's scroll area. Long values keep
//! a bounded preview and the same editor entity behind the common disclosure.

use gpui_kit::component::input::TextareaState;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::super::view::{DeclarativeView, WillResize};
use crate::ui::kit::Pressable as _;
use crate::ui::{fade::faded, format::editor_preview, motion};

pub(super) fn disclosure_id(id: &str) -> String {
    format!("input-preview:{id}")
}

/// This limits measured display rows, rather than allocating a fixed height.
pub(super) fn max_rows(value: &str, min_rows: usize, grow: bool) -> usize {
    if grow {
        value.chars().count().saturating_add(1).max(min_rows)
    } else {
        min_rows
    }
}

pub(super) fn resize(state: &Entity<TextareaState>, min_rows: usize, grow: bool, cx: &mut App) {
    let max = if grow {
        state
            .read(cx)
            .text()
            .chars()
            .count()
            .saturating_add(1)
            .max(min_rows)
    } else {
        min_rows
    };
    state.update(cx, |state, cx| state.set_auto_grow(min_rows, max, cx));
}

/// Only the visible prefix is copied, even for a very large saved value.
pub(super) fn preview(text: &str) -> Option<String> {
    editor_preview(text.chars())
}

pub(in crate::ui::extensions::declarative) fn render(
    view: &DeclarativeView,
    id: &str,
    grow: bool,
    state: &Entity<TextareaState>,
    area: impl IntoElement,
    cx: &Context<DeclarativeView>,
) -> AnyElement {
    let Some(preview) = grow
        .then(|| editor_preview(state.read(cx).text().chars()))
        .flatten()
    else {
        // The editor propagates wheel events when it fits, or at its edge.
        return area.into_any_element();
    };
    let id = disclosure_id(id);
    let open = view.fields.disclosures.get(&id).copied().unwrap_or(false);
    let key: SharedString = format!("extension-input-{}-{id}", cx.entity_id()).into();
    let (toggle, input_id) = (key.clone(), id.clone());
    let (keyboard_toggle, keyboard_id) = (key.clone(), id.clone());
    let focus = state.read(cx).focus_handle(cx);
    let keyboard_focus = focus.clone();
    let header = h_flex()
        .id(ElementId::Name(key.clone()))
        .w_full()
        .px_2()
        .py_1p5()
        .gap_2()
        .disclosure_surface(cx)
        // A collapsed editor keeps its ordinary focus stop on the header.
        // Opening mounts the textarea with that same handle and selection.
        .when(!open, |header| header.track_focus(&focus))
        .child(motion::chevron(
            ElementId::Name(key.clone()),
            open,
            cx.theme().muted_foreground,
        ))
        .child(faded(if open {
            "Свернуть"
        } else {
            "Развернуть для редактирования"
        }))
        .on_click(cx.listener(move |view, _, _, cx| {
            cx.emit(WillResize);
            view.fields.disclosures.insert(input_id.clone(), !open);
            motion::toggled(toggle.clone(), !open, cx);
            cx.notify();
        }))
        .on_key_down(cx.listener(move |view, event: &KeyDownEvent, window, cx| {
            if !open
                && keyboard_focus.is_focused(window)
                && !event.keystroke.modifiers.modified()
                && matches!(event.keystroke.key.as_str(), "enter" | "space")
            {
                cx.emit(WillResize);
                view.fields.disclosures.insert(keyboard_id.clone(), true);
                motion::toggled(keyboard_toggle.clone(), true, cx);
                cx.notify();
                cx.stop_propagation();
            }
        }));
    let preview = div()
        .px_2()
        .py_1()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(preview);
    let body = if open || motion::moving(&key, open, cx) {
        motion::preview_reveal(key, open, cx, preview, || area)
    } else {
        preview.into_any_element()
    };
    v_flex()
        .w_full()
        .min_w_0()
        .child(header)
        .child(body)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::max_rows;

    #[test]
    fn settings_growth_does_not_change_fixed_panel_height() {
        let value = "я".repeat(100_000);
        assert_eq!(max_rows(&value, 3, false), 3);
        assert_eq!(max_rows("a\nb", 3, true), 4);
    }
}
