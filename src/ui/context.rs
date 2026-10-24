// Адаптировано из Zed: crates/ui/src/components/context_menu.rs, crates/git_ui/src/git_ui.rs (shared menu and split-button trigger)
//! Right-click menus anchored to the element they belong to. gpui-kit's
//! `context_menu` opens at the pointer, and its popover clamps a menu into
//! the window without flipping, so near the bottom edge the menu covered
//! its own element. This one is placed like the dropdowns: below the
//! element, flipped above it when there is no room, sliding in the same way.

use super::{motion, prefs};
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AvailableSpace, Bounds, Context, DismissEvent, ElementId, Entity,
    Focusable as _, InteractiveElement, IntoElement, MouseButton, ParentElement as _, Pixels,
    Refineable as _, RenderOnce, SharedString, StyleRefinement, Styled, Window, canvas, div, px,
    size,
};

#[derive(Default)]
struct MenuState {
    bounds: Bounds<Pixels>,
    popup: motion::Popup<Entity<PopupMenu>>,
    measure_height: Option<Pixels>,
    /// Whether this pointer press began while the menu was still open.
    pressed_open: bool,
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
        let pressed = state.clone();
        let build = self.build;
        let measure_build = build.clone();
        let menu_open = state.read(cx).popup.is_open();
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
                    click(&click_state, &click_build, window, cx);
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
                    .child(main.rounded_r_none().border_r_0().pr_1().shadow_none())
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
                                click(&click_state, &click_build, window, cx);
                            }),
                    )
                    .into_any_element()
            }
        };
        let root = root
            .capture_any_mouse_down(move |event, _, cx| {
                if event.button == MouseButton::Left {
                    // PopupMenu dismisses on mouse-down outside its surface,
                    // before the trigger receives click on mouse-up. Preserve
                    // the press state so that same click cannot reopen it.
                    pressed.update(cx, |state, _| state.pressed_open = menu_open);
                }
            })
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
        let measuring = state.update(cx, |state, _| {
            state
                .measure_height
                .take()
                .zip(state.popup.current().cloned())
        });
        if let Some((max_height, menu)) = measuring {
            // Measure during layout, through the native view so stateful
            // captions keep their entity scope. Fitting menus need no scroll
            // area (including its padding overflow).
            let natural = menu.clone().into_any_element().layout_as_root(
                size(AvailableSpace::MaxContent, AvailableSpace::MaxContent),
                window,
                cx,
            );
            if natural.height > max_height {
                menu.update(cx, |menu, cx| {
                    menu.rebuild(window, cx, |menu, window, cx| {
                        measure_build(menu, window, cx).scrollable(true)
                    });
                });
            }
        }
        let shown = state.update(cx, |state, cx| {
            let frame = state.popup.frame(state.bounds, window, cx)?;
            Some((state.popup.value()?.clone(), frame))
        });
        let Some((menu, frame)) = shown else {
            return root;
        };
        // Native items already have 4 px of padding. Keep their rectangular
        // paint inside the shared corner even at the largest surface radius.
        let inset = (f32::from(prefs::current(cx).view.surface_radius) * 0.3)
            .ceil()
            .max(4.)
            - 4.;
        root.child(motion::popup(
            div().relative().p(prefs::scaled(inset)).child(menu),
            ElementId::Name(format!("{:?}:popup", self.id).into()),
            frame,
            cx,
        ))
    }
}

fn click(state: &Entity<MenuState>, build: &Build, window: &mut Window, cx: &mut App) {
    let pressed_open = state.update(cx, |state, _| std::mem::take(&mut state.pressed_open));
    if pressed_open && !window.last_input_was_keyboard() {
        if let Some(menu) = state.read(cx).popup.current().cloned() {
            menu.update(cx, |_, cx| cx.emit(DismissEvent));
        }
        return;
    }
    open(state, build, window, cx);
}

/// Builds the menu afresh, so its items reflect the moment it opens, and
/// retains only its inert drawing while dismissal animates.
fn open(state: &Entity<MenuState>, build: &Build, window: &mut Window, cx: &mut App) {
    if let Some(menu) = state.read(cx).popup.current().cloned() {
        menu.update(cx, |_, cx| cx.emit(DismissEvent));
        return;
    }
    let build = build.clone();
    let previous_focus = window.focused(cx);
    let margin = prefs::scaled(16.).to_pixels(window.rem_size());
    let room = window.viewport_size();
    let max_height = prefs::scaled(360.)
        .to_pixels(window.rem_size())
        .min((room.height - margin).max(px(0.)));
    let menu = PopupMenu::build(window, cx, move |menu, window, cx| {
        let menu = menu
            .appearance(false)
            .min_w(px(0.))
            .max_w(
                prefs::scaled(560.)
                    .to_pixels(window.rem_size())
                    .min((room.width - margin).max(px(0.))),
            )
            .max_h(max_height);
        build(menu, window, cx).scrollable(false)
    });
    let menu_focus = menu.focus_handle(cx);
    menu_focus.focus(window, cx);
    let generation = state.update(cx, |state, _| {
        state.measure_height = Some(max_height);
        state.popup.show(menu.clone())
    });
    let held = state.downgrade();
    window
        .subscribe(&menu, cx, move |_, _: &DismissEvent, window, cx| {
            let closed = held
                .update(cx, |state, _| state.popup.close_if(generation))
                .unwrap_or(false);
            if closed {
                motion::restore_focus(&menu_focus, previous_focus.clone(), window, cx);
                window.refresh();
            }
        })
        .detach();
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
