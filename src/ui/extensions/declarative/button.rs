//! The `button` node, a plain button or, with a menu, a split button whose
//! chevron part opens the entries, and the `menu` node. In a `bar` they are
//! compact: the extra small kit size, the nearest to the
//! `ButtonSize::Compact` buttons of Zed's panel footers.

use crate::ui::kit::ButtonLabel as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::menu::{self, Ask};
use super::view::DeclarativeView;
use crate::declarative::widget::{Node, Variant};
use crate::ui::kit;
use crate::ui::tip::Tip as _;

/// Where a button sits and so how it is sized.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Fit {
    /// The usual small button.
    Normal,
    /// Compact, at the end of a `bar`.
    Compact,
    /// Compact, at the start of a `bar`: its label gives way when the bar
    /// is short of room, and its menu opens upwards from its left edge.
    Shrink,
}

fn sized(button: Button, fit: Fit) -> Button {
    match fit {
        Fit::Normal => button.small(),
        Fit::Compact => button.xsmall(),
        Fit::Shrink => button.xsmall().flex_shrink(1.).min_w_0(),
    }
}

/// The `menu` node: a ghost button with a dropdown; only an unlabeled one
/// shows the default ellipsis.
pub(super) fn menu(
    node: &Node,
    path: &str,
    fit: Fit,
    cx: &mut Context<DeclarativeView>,
) -> AnyElement {
    let Node::Menu {
        label,
        icon,
        hint,
        items,
    } = node
    else {
        unreachable!("menu node expected");
    };
    let (items, view) = (items.clone(), cx.weak_entity());
    let icon = icon.as_deref().or(label.is_empty().then_some("ellipsis"));
    let button = kit::button(ElementId::Name(format!("decl-menu-{path}").into()));
    let button = sized(button, fit)
        .quiet(cx)
        .when(!label.is_empty(), |button| {
            button.faded_label(label.clone())
        })
        .when_some(hint.clone(), |button, hint| button.tip_text(hint))
        .when_some(icon, |button, icon| button.icon(super::icon(Some(icon))));
    let button = crate::ui::context::menu_button(
        ElementId::Name(format!("decl-dropdown-{path}").into()),
        button,
        !label.is_empty(),
        move |popup, _, _| menu::fill(popup, &items, &view),
    );
    if fit == Fit::Shrink {
        return div()
            .min_w_0()
            .overflow_hidden()
            .child(button)
            .into_any_element();
    }
    button.into_any_element()
}

pub(super) fn button(
    node: &Node,
    path: &str,
    fit: Fit,
    cx: &mut Context<DeclarativeView>,
) -> AnyElement {
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
    let button = kit::button(ElementId::Name(format!("decl-button-{path}").into()));
    let button = sized(button, fit)
        .when(!label.is_empty(), |button| {
            button.faded_label(label.clone())
        })
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
        Variant::Ghost => button.quiet(cx),
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
    let arrow = kit::button(ElementId::Name(format!("decl-arrow-{path}").into())).small();
    let arrow = match variant {
        Variant::Primary => arrow.primary(),
        Variant::Default => arrow.outline(),
        Variant::Ghost => arrow.quiet(cx),
        Variant::Danger => arrow.danger(),
    };
    crate::ui::context::split_button(
        ElementId::Name(format!("decl-split-{path}").into()),
        button,
        arrow,
        *disabled,
        move |popup, _, _| menu::fill(popup, &items, &view),
    )
    .into_any_element()
}
