// Адаптировано из Zed: crates/workspace/src/pane.rs (handle_tab_drop and dragged tab geometry)
//! Whole-row drag sorting. Settings and process widgets supply
//! identities and contents; gesture, preview, insertion geometry and motion
//! are shared. A drop can reorder the source list or use its explicit removal target.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

mod geometry;
mod hold;
mod removal;
#[cfg(test)]
mod tests;
pub(crate) use geometry::move_before;
use geometry::{Arrangement, Pending, keyboard_target, preview_order};
use hold::Hold;
use removal::{Removal, RemovalTarget};
#[cfg(test)]
pub(crate) use tests::ObservedBounds;

use super::kit::RowRule as _;
use super::{drag, fade, motion};

type Preview = Rc<dyn Fn(&mut App) -> Option<AnyElement>>;
type Move = Rc<dyn Fn(SharedString, Option<SharedString>, &mut Window, &mut App)>;

pub(crate) struct Row {
    id: SharedString,
    label: SharedString,
    child: AnyElement,
    preview: Option<Preview>,
}

impl Row {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            child: child.into_any_element(),
            preview: None,
        }
    }

    /// Rebuild the same row for the lifted copy; a removed owner falls back to its label.
    pub fn preview(mut self, build: impl Fn(&mut App) -> Option<AnyElement> + 'static) -> Self {
        self.preview = Some(Rc::new(build));
        self
    }
}

#[derive(IntoElement)]
pub(crate) struct List {
    id: ElementId,
    rows: Vec<Row>,
    moved: Move,
    revision: Option<u64>,
    separated: bool,
    drag_on_move: bool,
    removal: Option<Removal>,
}

impl List {
    pub fn new(
        id: impl Into<ElementId>,
        rows: Vec<Row>,
        moved: impl Fn(SharedString, Option<SharedString>, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            rows,
            moved: Rc::new(moved),
            revision: None,
            separated: false,
            drag_on_move: false,
            removal: None,
        }
    }

    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = Some(revision);
        self
    }

    /// Individually framed items keep their own surfaces, with an 8px gap
    /// instead of an inset rule. Animation and drop geometry use the same gap.
    pub fn separated(mut self) -> Self {
        self.separated = true;
        self
    }

    /// Compact items can start moving at the native pointer threshold.
    /// Rows with other actions keep their hold gesture.
    pub fn drag_on_move(mut self) -> Self {
        self.drag_on_move = true;
        self
    }

    /// Offer an explicit removal target during drag and Delete on focused rows.
    pub fn remove_on_drop(
        mut self,
        label: impl Into<SharedString>,
        apply: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.removal = Some(Removal {
            label: label.into(),
            apply: Rc::new(apply),
        });
        self
    }
}

/// Descendant switches/buttons/inputs opt out of their row's hold gesture.
/// Keep the normal click and keyboard action of the descendant intact.
pub(crate) fn controls(child: impl IntoElement) -> Div {
    div()
        .flex_none()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(child)
}

#[derive(Default)]
struct State {
    bounds: Bounds<Pixels>,
    rows: Vec<(SharedString, Bounds<Pixels>)>,
    pressed: Option<SharedString>,
    press_epoch: u64,
    hold: Option<Hold>,
    exclusions: Vec<Bounds<Pixels>>,
    armed: bool,
    timer: Option<Task<()>>,
    active: Option<SharedString>,
    active_hold: Option<Hold>,
    before: Option<SharedString>,
    over: bool,
    motion: Arrangement,
    focus: HashMap<SharedString, FocusHandle>,
    pending: Option<Pending>,
}

impl State {
    fn insertion_before(&self, position: Point<Pixels>) -> Option<SharedString> {
        self.rows
            .iter()
            .filter(|(id, _)| Some(id) != self.active.as_ref())
            .find(|(_, bounds)| position.y < bounds.center().y)
            .map(|(id, _)| id.clone())
    }

    fn release(&mut self) {
        self.pressed = None;
        self.hold = None;
        self.armed = false;
        self.timer = None;
    }

    fn arm(&mut self, epoch: u64, position: Point<Pixels>, active_window: bool) {
        if active_window
            && self.pressed.is_some()
            && self.press_epoch == epoch
            && self
                .hold
                .as_ref()
                .is_some_and(|hold| hold.contains(position))
        {
            self.armed = true;
        } else {
            self.release();
        }
    }

    fn cancel_motion(&mut self, dragging: bool, position: Point<Pixels>) -> bool {
        if self.pressed.is_some()
            && (!dragging
                || (!self.armed
                    && self
                        .hold
                        .as_ref()
                        .is_none_or(|hold| !hold.contains(position))))
        {
            self.release();
            true
        } else {
            false
        }
    }

    /// A native listener was selected during render, before prepaint can
    /// invalidate its hold. Never let that stale listener reach bubble.
    fn stale_drag_listener(&self, installed: Option<&SharedString>, epoch: u64) -> bool {
        installed.is_some_and(|id| {
            !self.armed
                || self.pressed.as_ref() != Some(id)
                || self.hold.is_none()
                || self.press_epoch != epoch
        })
    }
}

#[derive(Clone)]
struct Dragged {
    list: SharedString,
    id: SharedString,
    label: SharedString,
    width: Pixels,
    height: Pixels,
    preview: Option<Preview>,
    preview_shift: Point<Pixels>,
}

impl Render for Dragged {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self
            .preview
            .as_ref()
            .and_then(|preview| preview(cx))
            .unwrap_or_else(|| {
                div()
                    .px_3()
                    .py_2()
                    .text_sm()
                    .child(fade::faded(self.label.clone()))
                    .into_any_element()
            });
        // Keep a layout parent: relative insets on a layout root itself are
        // ignored, but its child can compensate the native layer's anchor.
        div().w(self.width).h(self.height).child(
            drag::lifted(
                div()
                    .w(self.width)
                    .min_w(self.width)
                    .h(self.height)
                    .overflow_hidden()
                    .child(div().size_full().child(content)),
                cx,
            )
            .relative()
            .left(self.preview_shift.x)
            .top(self.preview_shift.y),
        )
    }
}

impl RenderOnce for List {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| State::default());
        if state.read(cx).active.is_some() && !cx.has_active_drag() {
            state.update(cx, |state, _| {
                state.active = None;
                state.active_hold = None;
                state.over = false;
                state.release();
            });
        }
        if state.read(cx).pressed.is_some() && state.read(cx).press_epoch != drag::cancel_epoch(cx)
        {
            state.update(cx, |state, _| state.release());
        }
        let key: SharedString = format!("reorder:{:?}", state.entity_id()).into();
        let ids: Vec<_> = self.rows.iter().map(|row| row.id.clone()).collect();
        state.update(cx, |state, _| {
            if state
                .hold
                .as_ref()
                .is_some_and(|hold| !hold.retained(&ids, self.revision))
            {
                state.release();
            }
            if state
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.retained(&ids, self.revision))
            {
                state.pending = None;
            }
        });
        let stale = state
            .read(cx)
            .active_hold
            .as_ref()
            .is_some_and(|hold| !hold.retained(&ids, self.revision));
        if stale {
            if drag::current(cx).is_some_and(|source| source.starts_with(&format!("{key}:"))) {
                cx.stop_active_drag(window);
            }
            state.update(cx, |state, _| {
                state.active = None;
                state.active_hold = None;
                state.over = false;
                state.release();
            });
        }
        let installed_drag = state
            .read(cx)
            .pressed
            .clone()
            .filter(|_| state.read(cx).armed);
        let drag_on_move = self.drag_on_move;
        let children = self
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let armed = state.read(cx).armed
                    && state
                        .read(cx)
                        .pressed
                        .as_ref()
                        .is_some_and(|id| *id == row.id);
                let active = state.read(cx).active.as_ref() == Some(&row.id);
                let focus = state.update(cx, |state, cx| {
                    state
                        .focus
                        .entry(row.id.clone())
                        .or_insert_with(|| cx.focus_handle().tab_stop(true))
                        .clone()
                });
                let focused = focus.is_focused(window);
                let press_focus = focus.clone();
                let (keys, key_id, moved) = (state.clone(), row.id.clone(), self.moved.clone());
                let (press, id) = (state.clone(), row.id.clone());
                let revision = self.revision;
                let remove = self.removal.as_ref().map(|removal| removal.apply.clone());
                let label = row.label.clone();
                let data = Dragged {
                    list: key.clone(),
                    id: row.id.clone(),
                    label: row.label,
                    width: px(0.),
                    height: px(0.),
                    preview: row.preview,
                    preview_shift: point(px(0.), px(0.)),
                };
                div()
                    .id(ElementId::Name(row.id))
                    .when(!drag_on_move, |row| row.w_full())
                    .min_w_0()
                    .max_w_full()
                    .flex_none()
                    .relative()
                    .track_focus(&focus)
                    .when(remove.is_some(), |row| {
                        row.aria_label(label).aria_keyshortcuts("Delete")
                    })
                    .when(focused && super::kit::keyboard(), |row| {
                        row.child(super::selection::focus(cx))
                    })
                    .on_key_down(
                        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                            if !focus.is_focused(window) {
                                return;
                            }
                            let mods = event.keystroke.modifiers;
                            if !mods.modified()
                                && event.keystroke.key == "delete"
                                && let Some(remove) = &remove
                            {
                                if keys.read(cx).pending.is_none() && !cx.has_active_drag() {
                                    remove(key_id.clone(), window, cx);
                                }
                                cx.stop_propagation();
                                return;
                            }
                            if !mods.control || !mods.shift || mods.alt || mods.platform {
                                return;
                            }
                            let down = match event.keystroke.key.as_str() {
                                "up" => false,
                                "down" => true,
                                _ => return,
                            };
                            let target = keys.update(cx, |state, _| {
                                if state.pending.is_some() || state.active.is_some() {
                                    return None;
                                }
                                let source: Vec<_> =
                                    state.rows.iter().map(|(id, _)| id.clone()).collect();
                                let before = keyboard_target(&source, &key_id, down)?;
                                let order = preview_order(&source, Some(&key_id), before.as_ref());
                                state.pending = Some(Pending {
                                    source,
                                    order,
                                    revision,
                                });
                                Some(before)
                            });
                            if let Some(before) = target {
                                moved(key_id.clone(), before, window, cx);
                            }
                            cx.stop_propagation();
                        },
                    )
                    .when(index > 0 && !self.separated, |row| row.row_rule(cx))
                    .when(armed || drag_on_move, |row| row.cursor_grab())
                    .when(active, |row| row.opacity(drag::SOURCE_OPACITY))
                    .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                        if event.click_count != 1
                            || event.modifiers.modified()
                            || cx.has_active_drag()
                            || press.read(cx).pending.is_some()
                            || press
                                .read(cx)
                                .exclusions
                                .iter()
                                .any(|bounds| bounds.contains(&event.position))
                        {
                            return;
                        }
                        press_focus.focus(window, cx);
                        let epoch = drag::cancel_epoch(cx);
                        press.update(cx, |state, _| {
                            state.release();
                            let Some((_, bounds)) = state.rows.iter().find(|(row, _)| *row == id)
                            else {
                                return;
                            };
                            state.hold = Some(Hold::new(
                                event.position,
                                *bounds,
                                state.rows.iter().map(|(id, _)| id.clone()).collect(),
                                revision,
                            ));
                            state.pressed = Some(id.clone());
                            state.press_epoch = epoch;
                            state.armed = drag_on_move;
                        });
                        if drag_on_move {
                            return;
                        }
                        let weak = press.downgrade();
                        let task = window.spawn(cx, async move |cx| {
                            cx.background_executor().timer(motion::HOLD).await;
                            let _ = cx.update(|window, cx| {
                                let current_epoch = drag::cancel_epoch(cx);
                                let _ = weak.update(cx, |state, _| {
                                    state.arm(
                                        current_epoch,
                                        window.mouse_position(),
                                        window.is_window_active(),
                                    );
                                    window.refresh();
                                });
                            });
                        });
                        press.update(cx, |state, _| state.timer = Some(task));
                    })
                    // Establish native pending-mousedown state before the hold
                    // timer adds on_drag. Ordinary child clicks still bubble.
                    .on_click(|_, _, _| {})
                    .when(armed || drag_on_move, |row| {
                        let start = state.clone();
                        row.on_drag(data, move |data, native_offset, window, cx| {
                            let mut data = data.clone();
                            let epoch = drag::cancel_epoch(cx);
                            let started = start.update(cx, |state, _| {
                                if state.stale_drag_listener(Some(&data.id), epoch) {
                                    return false;
                                }
                                let hold = state.hold.take().unwrap();
                                data.preview_shift = hold.preview_shift(native_offset);
                                data.width = hold.bounds.size.width;
                                data.height = hold.bounds.size.height;
                                state.release();
                                state.active = Some(data.id.clone());
                                state.active_hold = Some(hold);
                                let position = window.mouse_position();
                                state.over = state.bounds.contains(&position);
                                // The first move starts native dragging before
                                // on_drag_move can run for this payload.
                                state.before = state.insertion_before(position);
                                true
                            });
                            if started {
                                drag::start(format!("{}:{}", data.list, data.id), cx);
                            } else {
                                // A pending native down can outlive the list's
                                // accepted press (for example after Escape).
                                window.defer(cx, |window, cx| {
                                    cx.stop_active_drag(window);
                                });
                            }
                            window.refresh();
                            cx.new(|_| data)
                        })
                    })
                    .child(row.child)
                    .into_any_element()
            })
            .collect();
        let hover = state.clone();
        let hover_key = key.clone();
        let removal = self
            .removal
            .filter(|_| state.read(cx).active.is_some())
            .map(RemovalTarget::new);
        let target = removal
            .as_ref()
            .map(|target| target.render(key.clone(), cx));

        div()
            .id(self.id)
            .w_full()
            .when(removal.is_some(), |list| list.flex().flex_col())
            .on_drag_move::<Dragged>(move |event, window, cx| {
                if event.drag(cx).list != hover_key {
                    return;
                }
                hover.update(cx, |state, _| {
                    let over = state.bounds.contains(&event.event.position);
                    let before = state.insertion_before(event.event.position);
                    if state.over != over || state.before != before {
                        state.over = over;
                        state.before = before;
                        window.refresh();
                    }
                });
            })
            .child(Arrange {
                ids,
                children,
                state,
                key,
                moved: self.moved,
                revision: self.revision,
                installed_drag,
                compact: drag_on_move,
                removal,
                gap: if self.separated {
                    super::prefs::scaled(8.).to_pixels(window.rem_size())
                } else {
                    px(0.)
                },
            })
            .children(target)
    }
}

struct Arrange {
    ids: Vec<SharedString>,
    children: Vec<AnyElement>,
    state: Entity<State>,
    key: SharedString,
    moved: Move,
    revision: Option<u64>,
    installed_drag: Option<SharedString>,
    compact: bool,
    removal: Option<RemovalTarget>,
    gap: Pixels,
}

impl IntoElement for Arrange {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Arrange {
    type RequestLayoutState = Vec<LayoutId>;
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Vec<LayoutId>) {
        let children: Vec<_> = self
            .children
            .iter_mut()
            .map(|child| child.request_layout(window, cx))
            .collect();
        let mut style = Style::default();
        style.display = Display::Flex;
        style.flex_direction = FlexDirection::Column;
        style.size.width = relative(1.).into();
        style.flex_shrink = 0.;
        if self.compact {
            style.align_items = Some(AlignItems::Start);
        }
        style.gap.height = self.gap.into();
        (window.request_layout(style, children.clone(), cx), children)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layouts: &mut Vec<LayoutId>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let raw: Vec<_> = self
            .ids
            .iter()
            .cloned()
            .zip(layouts.iter().map(|id| window.layout_bounds(*id)))
            .collect();
        let now = Instant::now();
        let animate = motion::moves(cx) && !cx.reduce_motion();
        let viewport = bounds.intersect(&window.content_mask().bounds);
        let positions = self.state.update(cx, |state, _| {
            state.bounds = viewport;
            state.rows = raw.clone();
            if state.hold.as_ref().is_some_and(|hold| {
                state
                    .rows
                    .iter()
                    .find(|(id, _)| Some(id) == state.pressed.as_ref())
                    .is_none_or(|(_, bounds)| *bounds != hold.bounds)
            }) {
                state.release();
            }
            state.exclusions.clear();
            let heights: HashMap<_, _> = raw
                .iter()
                .map(|(id, rect)| (id.clone(), rect.size.height))
                .collect();
            state.focus.retain(|id, _| heights.contains_key(id));
            let order = if state.active.is_some() && state.over {
                preview_order(&self.ids, state.active.as_ref(), state.before.as_ref())
            } else if let Some(pending) = &state.pending {
                pending.order.clone()
            } else {
                self.ids.clone()
            };
            let (shown, moving) = state.motion.layout(order, &heights, self.gap, now, animate);
            if moving {
                window.request_animation_frame();
            }
            shown
        });
        cx.default_global::<Scopes>().0.push(self.state.downgrade());
        for ((id, raw), child) in raw.iter().zip(&mut self.children) {
            let offset = bounds.top() + positions.get(id).copied().unwrap_or_default() - raw.top();
            window.with_element_offset(point(px(0.), offset), |window| {
                child.prepaint(window, cx);
            });
        }
        cx.default_global::<Scopes>().0.pop();
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Vec<LayoutId>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        // GPUI registers per-frame mouse listeners only during paint.
        let release = self.state.clone();
        let moved = self.moved.clone();
        let key = self.key.clone();
        let revision = self.revision;
        let removal = self.removal.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let ours =
                drag::current(cx).is_some_and(|source| source.starts_with(&format!("{key}:")));
            let dropped = ours && release.read(cx).bounds.contains(&event.position);
            let removal = removal
                .as_ref()
                .filter(|target| ours && target.contains(event.position, window));
            let active = release.read(cx).active.clone();
            // A platform may coalesce movement straight into mouse-up. The
            // release position remains authoritative even without another move.
            let before = release.read(cx).insertion_before(event.position);
            let armed = release.read(cx).armed;
            release.update(cx, |state, _| {
                state.release();
                if removal.is_none()
                    && dropped
                    && let Some(id) = &active
                {
                    let source: Vec<_> = state.rows.iter().map(|(id, _)| id.clone()).collect();
                    let order = preview_order(&source, Some(id), before.as_ref());
                    if source != order {
                        state.pending = Some(Pending {
                            source,
                            order,
                            revision,
                        });
                    }
                }
                if ours {
                    state.active = None;
                    state.active_hold = None;
                    state.over = false;
                }
            });
            if ours {
                cx.stop_active_drag(window);
                if let Some(id) = active {
                    if let Some(target) = removal {
                        (target.apply)(id, window, cx);
                    } else if dropped {
                        moved(id, before, window, cx);
                    }
                }
                // A drag release must never also click its source child.
                cx.stop_propagation();
            } else if armed {
                window.refresh();
            }
        });
        // Escape is intercepted before the keymap by the shared drag state.
        // Cancel a waiting hold before native bubble listeners see movement;
        // an armed/active drag may leave the source row normally.
        let cancel = self.state.clone();
        let installed_drag = self.installed_drag.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || cx.has_active_drag() {
                return;
            }
            let epoch = drag::cancel_epoch(cx);
            let block = cancel.update(cx, |state, _| {
                let cancelled = state.cancel_motion(event.dragging(), event.position);
                let stale = state.stale_drag_listener(installed_drag.as_ref(), epoch);
                if stale {
                    state.release();
                }
                if cancelled || stale {
                    window.refresh();
                }
                stale
            });
            if block {
                // GPUI's native handler still remembers the old mousedown;
                // it cannot reject construction and does not check buttons.
                cx.stop_propagation();
            }
        });
        for child in &mut self.children {
            child.paint(window, cx);
        }
    }
}

/// During prepaint an interactive descendant records its actual animated
/// bounds in the nearest enclosing list. No event-order dependency and no
/// bubbling/capture marker that another nested list could overwrite.
#[derive(Default)]
struct Scopes(Vec<WeakEntity<State>>);
impl Global for Scopes {}

/// Excludes a native interactive child from a containing row's hold
/// gesture without consuming its event or altering its layout/focus.
pub(crate) fn interactive(child: AnyElement) -> impl IntoElement {
    Interactive { child }
}

struct Interactive {
    child: AnyElement,
}
impl IntoElement for Interactive {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for Interactive {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.child.request_layout(window, cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let scope = cx
            .try_global::<Scopes>()
            .and_then(|scopes| scopes.0.last())
            .cloned();
        if let Some(scope) = scope {
            let clipped = bounds.intersect(&window.content_mask().bounds);
            let _ = scope.update(cx, |state, _| state.exclusions.push(clipped));
        }
        self.child.prepaint(window, cx);
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.paint(window, cx);
    }
}
