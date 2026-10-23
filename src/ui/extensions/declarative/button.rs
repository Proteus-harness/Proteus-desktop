//! The `button` node: a plain button or, with a menu, a split button whose
//! chevron part opens the entries.

use gpui_kit::component::button::{ButtonVariants as _, DropdownButton};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::menu::{self, Ask};
use super::view::DeclarativeView;
use crate::declarative::widget::{Node, Variant};
use crate::ui::kit;
use crate::ui::tip::Tip as _;

pub(super) fn button(node: &Node, path: &str, cx: &mut Context<DeclarativeView>) -> AnyElement {
    let Node::Button {
        label,
        action,
        value,
        variant,
        icon,
        selected,
        hint,
        confirm,
        prompt,
        menu: items,
        disabled,
    } = node
    else {
        unreachable!("button node expected");
    };
    let view = cx.weak_entity();
    let (sender, action, value) = (view.clone(), action.clone(), value.clone());
    let (confirm, prompt) = (confirm.clone(), prompt.clone());
    let button = kit::button(ElementId::Name(format!("decl-button-{path}").into()))
        .small()
        .when(!label.is_empty(), |button| button.label(label.clone()))
        .when_some(icon.as_deref(), |button, icon| {
            button.icon(super::icon(Some(icon)))
        })
        .selected(*selected)
        .disabled(*disabled)
        .on_click(move |_, window, cx| {
            let ask = Ask::of(confirm.as_ref(), prompt.as_ref());
            menu::send(&sender, &action, value.clone(), ask, window, cx)
        });
    let button = match variant {
        Variant::Primary => button.primary(),
        Variant::Default => button.outline(),
        Variant::Ghost => button.ghost(),
        Variant::Danger => button.danger(),
    };
    let button = match hint {
        Some(hint) => button.tip_text(hint.clone()),
        None => button,
    };
    if items.is_empty() {
        return button.into_any_element();
    }
    let items = items.clone();
    DropdownButton::new(ElementId::Name(format!("decl-split-{path}").into()))
        .button(button)
        .disabled(*disabled)
        .dropdown_menu_with_anchor(Anchor::TopRight, move |popup, _, _| {
            menu::fill(popup.min_w(px(180.)), &items, &view)
        })
        .into_any_element()
}
