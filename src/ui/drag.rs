//! What is being dragged right now, so its own place can fade while the
//! preview follows the cursor, and the look of that preview.

use crate::ui::kit::Frame as _;
use gpui_kit::component::*;
use gpui_kit::*;

/// Key of the dragged element, e.g. `tab:chat` or `badge:plan`.
#[derive(Default)]
struct DragSource {
    key: Option<SharedString>,
    press_window: Option<WindowId>,
    cancelled_release: Option<WindowId>,
    epoch: u64,
}

impl Global for DragSource {}

pub(super) fn init(cx: &mut App) {
    // GPUI resolves key bindings before raw key listeners. Intercept once,
    // before an input, disclosure or popup can turn Escape into an action.
    cx.intercept_keystrokes(|event, window, cx| {
        if event.keystroke.key == "escape"
            && !event.keystroke.modifiers.modified()
            && cancel(window, cx)
        {
            cx.stop_propagation();
        }
    })
    .detach();
}

/// Remembers the element a drag starts from; call in the drag constructor.
pub(super) fn start(key: impl Into<SharedString>, cx: &mut App) {
    let state = cx.default_global::<DragSource>();
    state.key = Some(key.into());
    state.cancelled_release = None;
}

/// Whether the element with `key` is being dragged.
pub(super) fn is_source(key: &str, cx: &App) -> bool {
    current(cx).is_some_and(|source| source == key)
}

/// Key of what is being dragged, if anything.
pub(super) fn current(cx: &App) -> Option<&str> {
    if !cx.has_active_drag() {
        return None;
    }
    cx.try_global::<DragSource>()?.key.as_deref()
}

/// Cancel before a focused child handles Escape. This also invalidates
/// hold timers that have not started a native drag yet.
pub(super) fn cancel(window: &mut Window, cx: &mut App) -> bool {
    let window_id = window.window_handle().window_id();
    let pressed = cx
        .try_global::<DragSource>()
        .is_some_and(|state| state.press_window == Some(window_id));
    let stopped = cx.stop_active_drag(window);
    if !stopped && !pressed {
        return false;
    }
    let state = cx.default_global::<DragSource>();
    state.key = None;
    state.press_window = None;
    state.epoch = state.epoch.wrapping_add(1);
    state.cancelled_release = Some(window_id);
    window.refresh();
    true
}

pub(super) fn cancel_epoch(cx: &App) -> u64 {
    cx.try_global::<DragSource>().map_or(0, |state| state.epoch)
}

/// A release after Escape belongs to the cancelled drag, not to a click
/// on the source child. The next press starts an ordinary new gesture.
pub(super) fn begin_press(window: &Window, cx: &mut App) {
    let state = cx.default_global::<DragSource>();
    // A new native drag may belong to a control that does not need a
    // source marker. Never lend it the previous gesture's source key.
    state.key = None;
    state.press_window = Some(window.window_handle().window_id());
    state.cancelled_release = None;
}

pub(super) fn discard_release(window: &Window, cx: &mut App) -> bool {
    let window_id = window.window_handle().window_id();
    let state = cx.default_global::<DragSource>();
    if state.press_window == Some(window_id) {
        state.press_window = None;
    }
    if state.cancelled_release == Some(window_id) {
        state.cancelled_release = None;
        return true;
    }
    false
}

/// Finish pointer gestures before a closing popup can consume MouseUp. Keep
/// text highlighted while stopping its drag; recover outside-window releases
/// on the next move without buttons. No hitbox or layout space is added.
pub(super) fn release_boundary() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |_, (), window, _| {
            window.on_mouse_event(|event: &MouseUpEvent, phase, window, cx| {
                if phase.capture() && event.button == MouseButton::Left {
                    gpui_kit::base::TextSelection::end(window, cx);
                    if discard_release(window, cx) {
                        cx.stop_propagation();
                    }
                }
            });
            window.on_mouse_event(|event: &MouseMoveEvent, phase, window, cx| {
                if phase.capture() && event.pressed_button.is_none() {
                    gpui_kit::base::TextSelection::end(window, cx);
                }
            });
        },
    )
    .absolute()
    .inset_0()
}

/// Opacity of the place an element was lifted from.
pub(super) const SOURCE_OPACITY: f32 = 0.35;

/// Lifted look of a dragged copy: raised above the content with a shadow.
pub(super) fn lifted(element: Div, cx: &App) -> Div {
    let theme = cx.theme();
    element
        .frame(cx)
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .shadow_lg()
        .opacity(0.95)
}
