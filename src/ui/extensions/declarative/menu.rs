//! Menus and confirmations of declarative controls.

use super::view::DeclarativeView;
use crate::declarative::control::{Confirm, MenuItem};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

/// Sends `action`, asking first when `confirm` is set.
pub(super) fn send(
    view: &WeakEntity<DeclarativeView>,
    action: &str,
    value: Option<Value>,
    confirm: Option<&Confirm>,
    window: &mut Window,
    cx: &mut App,
) {
    let (view, action) = (view.clone(), action.to_string());
    let Some(confirm) = confirm.cloned() else {
        view.update(cx, |view, cx| view.act(action, value, cx)).ok();
        return;
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
        let (view, entry) = (view.clone(), item.clone());
        let mut row = PopupMenuItem::new(item.label.clone()).on_click(move |_, window, cx| {
            send(
                &view,
                &entry.action,
                entry.value.clone(),
                entry.confirm.as_ref(),
                window,
                cx,
            )
        });
        if let Some(icon) = item.icon.as_deref() {
            row = row.icon(super::icon(Some(icon)));
        }
        menu.item(row)
    })
}
