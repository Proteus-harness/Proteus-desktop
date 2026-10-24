//! A small dialog that asks for one line of text before an action is sent.

use std::cell::Cell;
use std::rc::Rc;

use super::view::DeclarativeView;
use crate::declarative::control::Prompt;
use crate::ui::kit;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::*;
use serde_json::{Value, json};

/// Opens the prompt; confirming sends `action` with `{value, text}`.
pub(super) fn open(
    view: WeakEntity<DeclarativeView>,
    action: String,
    value: Option<Value>,
    prompt: Prompt,
    window: &mut Window,
    cx: &mut App,
) {
    let initial = prompt.value.clone().unwrap_or_default();
    let selected = match initial.rfind('.') {
        Some(dot) if prompt.stem && dot > 0 => 0..dot,
        _ => 0..initial.len(),
    };
    let input = cx.new(|cx| {
        let mut input =
            InputState::new(window, cx).placeholder(prompt.placeholder.clone().unwrap_or_default());
        input.set_value(initial, window, cx);
        input.set_selected_range(selected, cx);
        input
    });
    // Enter may reach both the field and the dialog; the first one sends.
    let sent = Rc::new(Cell::new(false));
    type Submit = Rc<dyn Fn(&Entity<InputState>, &mut App) -> bool>;
    let submit: Submit = Rc::new(move |input, cx| {
        if sent.replace(true) {
            return false;
        }
        let text = input.read(cx).value().to_string();
        let value = json!({"value": value.clone(), "text": text});
        view.update(cx, |view, cx| view.act(action.clone(), Some(value), cx))
            .ok();
        true
    });
    let on_enter = submit.clone();
    window
        .subscribe(&input, cx, move |input, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) && on_enter(&input, cx) {
                window.close_dialog(cx);
            }
        })
        .detach();
    let field = input.clone();
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (input, submit) = (field.clone(), submit.clone());
        alert
            .title(prompt.title.clone())
            .child(kit::input(&field).w_full())
            .ok_text(prompt.ok.clone().unwrap_or_else(|| "OK".into()))
            .show_cancel(true)
            .cancel_text("Отмена")
            .on_ok(move |_, _, cx| {
                submit(&input, cx);
                true
            })
    });
    window.defer(cx, move |window, cx| {
        input.update(cx, |input, cx| input.focus(window, cx));
    });
}
