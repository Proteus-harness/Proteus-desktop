// Адаптировано из Zed: crates/workspace/src/pane.rs (handle_tab_drop and dragged tab geometry)
//! Whole-row hold-and-drag sorting. Settings and process widgets supply
//! identities and contents; gesture, preview, insertion geometry and motion
//! are shared. Data changes only on a successful drop inside the same list.

use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

mod geometry;
pub(crate) use geometry::move_before;
use geometry::{Pending, Position, keyboard_target, preview_order};

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
        }
    }

    pub fn revision(mut self, revision: u64) -> Self {
        self.revision = Some(revision);
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
    exclusions: Vec<Bounds<Pixels>>,
    armed: bool,
    timer: Option<Task<()>>,
    active: Option<SharedString>,
    before: Option<SharedString>,
    over: bool,
    motion: HashMap<SharedString, Position>,
    focus: HashMap<SharedString, FocusHandle>,
    pending: Option<Pending>,
}

impl State {
    fn release(&mut self) {
        self.pressed = None;
        self.armed = false;
        self.timer = None;
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
        drag::lifted(
            div()
                .w(self.width)
                .min_w(self.width)
                .h(self.height)
                .overflow_hidden()
                .child(div().size_full().child(content)),
            cx,
        )
    }
}

impl RenderOnce for List {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| State::default());
        if state.read(cx).active.is_some() && !cx.has_active_drag() {
            state.update(cx, |state, _| {
                state.active = None;
                state.over = false;
                state.release();
            });
        }
        if state.read(cx).pressed.is_some() && state.read(cx).press_epoch != drag::cancel_epoch(cx)
        {
            state.update(cx, |state, _| state.release());
        }
        let key: SharedString = format!("reorder:{:?}", self.id).into();
        let ids: Vec<_> = self.rows.iter().map(|row| row.id.clone()).collect();
        state.update(cx, |state, _| {
            if state
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.retained(&ids, self.revision))
            {
                state.pending = None;
            }
        });
        let missing = state
            .read(cx)
            .active
            .as_ref()
            .is_some_and(|id| !ids.contains(id));
        if missing {
            if drag::current(cx).is_some_and(|source| source.starts_with(&format!("{key}:"))) {
                cx.stop_active_drag(window);
            }
            state.update(cx, |state, _| {
                state.active = None;
                state.over = false;
                state.release();
            });
        }
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
                let (keys, key_id, moved, revision) = (
                    state.clone(),
                    row.id.clone(),
                    self.moved.clone(),
                    self.revision,
                );
                let (press, id) = (state.clone(), row.id.clone());
                let data = Dragged {
                    list: key.clone(),
                    id: row.id.clone(),
                    label: row.label,
                    width: px(0.),
                    height: px(0.),
                    preview: row.preview,
                };
                div()
                    .id(ElementId::Name(row.id))
                    .w_full()
                    .flex_none()
                    .relative()
                    .track_focus(&focus)
                    .when(focused && super::kit::keyboard(), |row| {
                        row.child(super::selection::focus(cx))
                    })
                    .on_key_down(
                        move |event: &KeyDownEvent, window: &mut Window, cx: &mut App| {
                            let mods = event.keystroke.modifiers;
                            if !focus.is_focused(window)
                                || !mods.control
                                || !mods.shift
                                || mods.alt
                                || mods.platform
                            {
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
                    .when(index > 0, |row| row.row_rule(cx))
                    .when(armed, |row| row.cursor_grab())
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
                            state.pressed = Some(id.clone());
                            state.press_epoch = epoch;
                        });
                        let weak = press.downgrade();
                        let task = window.spawn(cx, async move |cx| {
                            cx.background_executor().timer(motion::HOLD).await;
                            let _ = cx.update(|window, cx| {
                                if !window.is_window_active() {
                                    return;
                                }
                                let current_epoch = drag::cancel_epoch(cx);
                                let _ = weak.update(cx, |state, _| {
                                    if state.pressed.is_some() && state.press_epoch == current_epoch
                                    {
                                        state.armed = true;
                                        window.refresh();
                                    }
                                });
                            });
                        });
                        press.update(cx, |state, _| state.timer = Some(task));
                    })
                    // Establish native pending-mousedown state before the hold
                    // timer adds on_drag. Ordinary child clicks still bubble.
                    .on_click(|_, _, _| {})
                    .when(armed, |row| {
                        let start = state.clone();
                        row.on_drag(data, move |data, _, window, cx| {
                            let mut data = data.clone();
                            start.update(cx, |state, _| {
                                if let Some((_, bounds)) =
                                    state.rows.iter().find(|(id, _)| *id == data.id)
                                {
                                    data.width = bounds.size.width;
                                    data.height = bounds.size.height;
                                }
                                state.release();
                                state.active = Some(data.id.clone());
                                state.over = true;
                                state.before = state
                                    .rows
                                    .iter()
                                    .position(|(id, _)| *id == data.id)
                                    .and_then(|at| state.rows.get(at + 1))
                                    .map(|(id, _)| id.clone());
                            });
                            drag::start(format!("{}:{}", data.list, data.id), cx);
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

        div()
            .id(self.id)
            .w_full()
            .on_drag_move::<Dragged>(move |event, window, cx| {
                if event.drag(cx).list != hover_key {
                    return;
                }
                hover.update(cx, |state, _| {
                    let over = event.bounds.contains(&event.event.position);
                    let before = state
                        .rows
                        .iter()
                        .filter(|(id, _)| Some(id) != state.active.as_ref())
                        .find(|(_, bounds)| event.event.position.y < bounds.center().y)
                        .map(|(id, _)| id.clone());
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
            })
    }
}

struct Arrange {
    ids: Vec<SharedString>,
    children: Vec<AnyElement>,
    state: Entity<State>,
    key: SharedString,
    moved: Move,
    revision: Option<u64>,
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
        let positions = self.state.update(cx, |state, _| {
            state.bounds = bounds;
            state.rows = raw.clone();
            state.exclusions.clear();
            state.motion.retain(|id, _| self.ids.contains(id));
            state.focus.retain(|id, _| self.ids.contains(id));
            let heights: HashMap<_, _> = raw
                .iter()
                .map(|(id, rect)| (id.clone(), rect.size.height))
                .collect();
            let order = if state.active.is_some() && state.over {
                preview_order(&self.ids, state.active.as_ref(), state.before.as_ref())
            } else if let Some(pending) = &state.pending {
                pending.order.clone()
            } else {
                self.ids.clone()
            };
            let mut top = px(0.);
            let mut shown = HashMap::new();
            for id in order {
                let position = state.motion.entry(id.clone()).or_insert(Position {
                    from: top,
                    target: top,
                    tween: motion::Tween::start(motion::BASE),
                });
                if position.target != top {
                    position.from = position.sample(now);
                    position.target = top;
                    position.tween = motion::Tween::start(motion::BASE);
                }
                let y = if motion::moves() {
                    position.sample(now)
                } else {
                    top
                };
                if y != top {
                    window.request_animation_frame();
                }
                shown.insert(id.clone(), y);
                top += heights.get(&id).copied().unwrap_or_default();
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
        let release = self.state.clone();
        let moved = self.moved.clone();
        let key = self.key.clone();
        let revision = self.revision;
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
                return;
            }
            let ours =
                drag::current(cx).is_some_and(|source| source.starts_with(&format!("{key}:")));
            let dropped = ours && release.read(cx).bounds.contains(&event.position);
            let active = release.read(cx).active.clone();
            let before = release.read(cx).before.clone();
            let armed = release.read(cx).armed;
            release.update(cx, |state, _| {
                state.release();
                if dropped && let Some(id) = &active {
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
                    state.over = false;
                }
            });
            if ours {
                cx.stop_active_drag(window);
                if dropped && let Some(id) = active {
                    moved(id, before, window, cx);
                }
                // A drag release must never also click its source child.
                cx.stop_propagation();
            } else if armed {
                window.refresh();
            }
        });
        // Escape is intercepted before the keymap by the shared drag state.
        // Movement during HOLD must not permanently disarm the same press:
        // touchpad jitter and beginning to move before the timer are normal.
        let cancel = self.state.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || cx.has_active_drag() {
                return;
            }
            cancel.update(cx, |state, _| {
                if state.pressed.is_some() && !event.dragging() {
                    state.release();
                }
            });
        });
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
