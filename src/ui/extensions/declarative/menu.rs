//! Menus, confirmations and prompts of declarative controls.

use super::view::DeclarativeView;
use crate::declarative::control::{Confirm, MenuItem, Prompt};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

/// What a control asks before it sends its action.
#[derive(Clone, Copy)]
pub(super) enum Ask<'a> {
    Nothing,
    Confirm(&'a Confirm),
    Prompt(&'a Prompt),
}

impl<'a> Ask<'a> {
    pub fn of(confirm: Option<&'a Confirm>, prompt: Option<&'a Prompt>) -> Self {
        match (confirm, prompt) {
            (Some(confirm), _) => Ask::Confirm(confirm),
            (None, Some(prompt)) => Ask::Prompt(prompt),
            (None, None) => Ask::Nothing,
        }
    }
}

/// Sends `action`, asking first as `ask` says.
pub(super) fn send(
    view: &WeakEntity<DeclarativeView>,
    action: &str,
    value: Option<Value>,
    ask: Ask,
    window: &mut Window,
    cx: &mut App,
) {
    let (view, action) = (view.clone(), action.to_string());
    let confirm = match ask {
        Ask::Nothing => {
            view.update(cx, |view, cx| view.act(action, value, cx)).ok();
            return;
        }
        Ask::Prompt(prompt) => {
            return super::prompt::open(view, action, value, prompt.clone(), window, cx);
        }
        Ask::Confirm(confirm) => confirm.clone(),
    };
    window.open_alert_dialog(cx, move |alert, _, _| {
        let (view, action, value) = (view.clone(), action.clone(), value.clone());
        alert
            .title(confirm.title.clone())
            .when_some(confirm.message.clone(), |alert, message| {
                alert.description(message)
            })
            .ok_text(confirm.ok.clone().unwrap_or_else(|| "Да".into()))
            .ok_variant(if confirm.danger {
                ButtonVariant::Danger
            } else {
                ButtonVariant::Primary
            })
            .show_cancel(true)
            .cancel_text("Отмена")
            .on_ok(move |_, _, cx| {
                let (action, value) = (action.clone(), value.clone());
                view.update(cx, |view, cx| view.act(action, value, cx)).ok();
                true
            })
    });
}

/// Fills a popup menu with the process's entries.
pub(super) fn fill(
    menu: PopupMenu,
    items: &[MenuItem],
    view: &WeakEntity<DeclarativeView>,
) -> PopupMenu {
    items.iter().fold(menu, |menu, item| {
        let menu = if item.separated {
            menu.separator()
        } else {
            menu
        };
        if item.header {
            return menu.item(PopupMenuItem::label(item.label.clone()));
        }
        let (view, entry) = (view.clone(), item.clone());
        let row = if item.danger && !item.disabled {
            let label = item.label.clone();
            PopupMenuItem::element(move |_, cx| {
                div().text_color(cx.theme().danger).child(label.clone())
            })
        } else {
            PopupMenuItem::new(item.label.clone())
        };
        let row = row
            .disabled(item.disabled)
            .when_some(item.checked, |row, checked| row.checked(checked))
            .when_some(item.icon.as_deref(), |row, icon| {
                row.icon(super::icon(Some(icon)))
            })
            .on_click(move |_, window, cx| {
                let ask = Ask::of(entry.confirm.as_ref(), entry.prompt.as_ref());
                send(&view, &entry.action, entry.value.clone(), ask, window, cx)
            });
        menu.item(row)
    })
}
