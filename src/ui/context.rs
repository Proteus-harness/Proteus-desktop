//! Right-click menus anchored to the element they belong to. gpui-kit's
//! `context_menu` opens at the pointer, and its popover clamps a menu into
//! the window without flipping, so near the bottom edge the menu covered
//! its own element. This one is placed like the dropdowns: below the
//! element, flipped above it when there is no room, sliding in the same way.

use super::{motion, prefs};
use gpui_kit::base::{Align, POPUP_PRIORITY, Placement, Positioner};
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Bounds, Context, DismissEvent, ElementId, Entity, Focusable as _,
    InteractiveElement, IntoElement, MouseButton, ParentElement as _, Pixels, Refineable as _,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, canvas, deferred, div, px,
};

#[derive(Default)]
struct MenuState {
    bounds: Bounds<Pixels>,
    menu: Option<Entity<PopupMenu>>,
    generation: usize,
}

type Build = std::rc::Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// An element with a menu that opens below it on a right click.
#[derive(IntoElement)]
pub(crate) struct ContextMenu {
    id: ElementId,
    trigger: Trigger,
    build: Build,
    style: StyleRefinement,
}

enum Trigger {
    Element(AnyElement),
    Button(Button, bool),
    Split(Button, Button, bool),
}

/// A dropdown anchored to the entire button, sharing context-menu motion.
pub(crate) fn menu_button(
    id: impl Into<ElementId>,
    button: Button,
    caret: bool,
    build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> ContextMenu {
    ContextMenu {
        id: id.into(),
        trigger: Trigger::Button(button, caret),
        build: std::rc::Rc::new(build),
        style: StyleRefinement::default(),
    }
}

/// Joined action and dropdown halves. Either the chevron or a right click
/// opens the same menu at the bounds of the entire control.
pub(crate) fn split_button(
    id: impl Into<ElementId>,
    main: Button,
    arrow: Button,
    disabled: bool,
    build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> ContextMenu {
    ContextMenu {
        id: id.into(),
        trigger: Trigger::Split(main, arrow, disabled),
        build: std::rc::Rc::new(build),
        style: StyleRefinement::default(),
    }
}

impl ContextMenu {
    /// Styles the wrapper that takes part in the parent layout, where
    /// `w_full` or `flex_none` of the element must be repeated.
    pub(crate) fn trigger_style(mut self, style: StyleRefinement) -> Self {
        self.style.refine(&style);
        self
    }
}

impl Styled for ContextMenu {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ContextMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| MenuState::default());
        let mut root = div().id(self.id.clone()).relative();
        root.style().refine(&self.style);
        let measured = state.clone();
        let opened = state.clone();
        let build = self.build;
        let menu_open = state.read(cx).menu.is_some();
        let disabled = matches!(&self.trigger, Trigger::Split(_, _, true));
        let (click_state, click_build) = (state.clone(), build.clone());
        let trigger = match self.trigger {
            Trigger::Element(element) => element,
            Trigger::Button(button, caret) => button
                .when(caret, |b| {
                    b.child(motion::menu_chevron(
                        ElementId::Name(format!("{:?}:chevron", self.id).into()),
                        menu_open,
                        cx.theme().muted_foreground,
                    ))
                })
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    open(&click_state, &click_build, window, cx);
                })
                .into_any_element(),
            Trigger::Split(main, arrow, disabled) => {
                let chevron = motion::menu_chevron(
                    ElementId::Name(format!("{:?}:chevron", self.id).into()),
                    menu_open,
                    cx.theme().muted_foreground,
                );
                h_flex()
                    .min_w_0()
                    .group("split-control")
                    .child(main.rounded_r_none().border_r_0().shadow_none())
                    .child(
                        arrow
                            .rounded_l_none()
                            .shadow_none()
                            .min_h(prefs::scaled(26.))
                            .h(prefs::scaled(26.))
                            .w(prefs::scaled(26.))
                            .px_0()
                            .disabled(disabled)
                            .selected(menu_open)
                            .child(chevron)
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                open(&click_state, &click_build, window, cx);
                            }),
                    )
                    .into_any_element()
            }
        };
        let root = root
            .child(trigger)
            .child(
                canvas(
                    move |bounds, _, cx| measured.update(cx, |state, _| state.bounds = bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                cx.stop_propagation();
                if !disabled {
                    open(&opened, &build, window, cx);
                }
            });
        let (bounds, menu) = {
            let state = state.read(cx);
            (state.bounds, state.menu.clone())
        };
        let Some(menu) = menu else {
            return root;
        };
        let generation = state.read(cx).generation;
        let animation = ElementId::Name(format!("{:?}:enter:{generation}", self.id).into());
        let surface = div().relative().child(menu);
        let surface = motion::popup(surface, animation, cx);
        root.child(
            deferred(
                Positioner::side(bounds)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(px(4.))
                    .margin(px(8.))
                    .occlude()
                    .child(surface),
            )
            .with_priority(POPUP_PRIORITY),
        )
    }
}

/// Builds the menu afresh, so its items reflect the moment it opens, and
/// forgets it once dismissed.
fn open(state: &Entity<MenuState>, build: &Build, window: &mut Window, cx: &mut App) {
    if let Some(menu) = state.read(cx).menu.clone() {
        menu.update(cx, |_, cx| cx.emit(DismissEvent));
        return;
    }
    let build = build.clone();
    let previous_focus = window.focused(cx);
    let menu = PopupMenu::build(window, cx, move |menu, window, cx| build(menu, window, cx));
    let menu_focus = menu.focus_handle(cx);
    menu_focus.focus(window, cx);
    let held = state.downgrade();
    window
        .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
            if menu_focus.contains_focused(window, cx)
                && let Some(previous) = &previous_focus
            {
                previous.focus(window, cx);
            }
            _ = held.update(cx, |state, _| state.menu = None);
            window.refresh();
        })
        .detach();
    state.update(cx, |state, _| {
        state.menu = Some(menu);
        state.generation += 1;
    });
    window.refresh();
}

pub(crate) trait RightClickMenu: InteractiveElement + IntoElement + Sized + 'static {
    /// Opens the menu `build` fills below this element on a right click.
    #[track_caller]
    fn right_click_menu(
        mut self,
        build: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> ContextMenu {
        let caller = std::panic::Location::caller();
        let id: SharedString = match self.interactivity().element_id.clone() {
            Some(id) => format!("context-menu:{id:?}").into(),
            None => format!("context-menu:{caller}").into(),
        };
        ContextMenu {
            id: ElementId::Name(id),
            trigger: Trigger::Element(self.into_any_element()),
            build: std::rc::Rc::new(build),
            style: StyleRefinement::default(),
        }
    }
}

impl<E: InteractiveElement + IntoElement + Sized + 'static> RightClickMenu for E {}
