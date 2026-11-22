//! A separate hitbox for removing an ordered row on mouse release.

use std::{cell::RefCell, rc::Rc};

use gpui_kit::accesskit::Role;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::*;

use super::Dragged;

type Remove = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

pub(super) struct Removal {
    pub label: SharedString,
    pub apply: Remove,
}

#[derive(Clone)]
pub(super) struct RemovalTarget {
    label: SharedString,
    pub apply: Remove,
    hitbox: Rc<RefCell<Option<Hitbox>>>,
}

impl RemovalTarget {
    pub fn new(removal: Removal) -> Self {
        Self {
            label: removal.label,
            apply: removal.apply,
            hitbox: Rc::default(),
        }
    }

    pub fn contains(&self, position: Point<Pixels>, window: &Window) -> bool {
        self.hitbox
            .borrow()
            .as_ref()
            .is_some_and(|hitbox| hitbox.is_hovered_at(position, window))
    }

    pub fn render(&self, list: SharedString, cx: &App) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let hitbox = self.hitbox.clone();
        h_flex()
            .id("reorder-remove-drop")
            .role(Role::Region)
            .aria_label(self.label.clone())
            .relative()
            .mt_2()
            .min_h_8()
            .px_2()
            .gap_2()
            .rounded(theme.radius)
            .border_1()
            .border_dashed()
            .border_color(theme.danger)
            .text_sm()
            .text_color(theme.danger)
            .drag_over::<Dragged>(move |style, dragged, _, cx| {
                if dragged.list == list {
                    style.bg(cx.theme().danger.opacity(0.15))
                } else {
                    style
                }
            })
            .child(Icon::new(Lucide::Trash).xsmall())
            .child(self.label.clone())
            .child(
                canvas(
                    move |bounds, window, _cx| {
                        #[cfg(test)]
                        super::ObservedBounds::record("reorder-remove-drop", bounds, _cx);
                        *hitbox.borrow_mut() =
                            Some(window.insert_hitbox(bounds, HitboxBehavior::Normal));
                    },
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            )
    }
}
