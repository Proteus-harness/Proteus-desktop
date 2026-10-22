//! Right-click menus anchored to the element they belong to. gpui-kit's
//! `context_menu` opens at the pointer; this opens below the element, through
//! the same popover as the dropdown menus, so tooltips, dropdowns and context
//! menus all appear at the element with one animation.

use std::rc::Rc;

use gpui_kit::base::Selectable;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::popover::Popover;
use gpui_kit::{
    Anchor, AnyElement, App, Context, DismissEvent, ElementId, Entity, Focusable as _,
    InteractiveElement, IntoElement, MouseButton, RenderOnce, SharedString, Window,
};

type Build = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

#[derive(Default)]
struct MenuState {
    menu: Option<Entity<PopupMenu>>,
}

/// The element a context menu opens from; it does not change while open.
#[derive(IntoElement)]
struct Trigger {
    element: AnyElement,
    open: bool,
}

impl Selectable for Trigger {
    fn selected(mut self, selected: bool) -> Self {
        self.open = selected;
        self
    }

    fn is_selected(&self) -> bool {
        self.open
    }
}

impl RenderOnce for Trigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.element
    }
}

pub(crate) trait RightClickMenu: InteractiveElement + IntoElement + Sized + 'static {
    /// Opens the menu `build` fills below this element on a right click.
    #[track_caller]
    fn right_click_menu(
        mut self,
        build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Popover {
        let caller = std::panic::Location::caller();
        let id: SharedString = match self.interactivity().element_id.clone() {
            Some(id) => format!("context-menu:{id:?}").into(),
            None => format!("context-menu:{caller}").into(),
        };
        let trigger = Trigger {
            element: self.into_any_element(),
            open: false,
        };
        let build: Build = Rc::new(build);
        let state_id = ElementId::Name(format!("{id}:state").into());
        Popover::new(ElementId::Name(id))
            .appearance(false)
            .overlay_closable(false)
            .mouse_button(MouseButton::Right)
            .anchor(Anchor::TopLeft)
            .trigger(trigger)
            .content(move |_, window, cx| {
                let state =
                    window.use_keyed_state(state_id.clone(), cx, |_, _| MenuState::default());
                if let Some(menu) = state.read(cx).menu.clone() {
                    return menu;
                }
                let build = build.clone();
                let menu =
                    PopupMenu::build(window, cx, move |menu, window, cx| build(menu, window, cx));
                state.update(cx, |state, _| state.menu = Some(menu.clone()));
                menu.focus_handle(cx).focus(window, cx);
                let popover = cx.entity().downgrade();
                let held = state.downgrade();
                window
                    .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
                        if let Some(popover) = popover.upgrade() {
                            popover.update(cx, |popover, cx| popover.dismiss(window, cx));
                        }
                        _ = held.update(cx, |state, _| state.menu = None);
                    })
                    .detach();
                menu
            })
    }
}

impl<E: InteractiveElement + IntoElement + Sized + 'static> RightClickMenu for E {}
