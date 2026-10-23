//! What is being dragged right now, so its own place can fade while the
//! preview follows the cursor, and the look of that preview.

use gpui_kit::component::*;
use gpui_kit::*;

/// Key of the dragged element, e.g. `tab:chat` or `badge:plan`.
#[derive(Default)]
struct DragSource {
    key: Option<SharedString>,
    cancelled_release: bool,
    epoch: u64,
}

impl Global for DragSource {}

/// Remembers the element a drag starts from; call in the drag constructor.
pub(super) fn start(key: impl Into<SharedString>, cx: &mut App) {
    let epoch = cancel_epoch(cx);
    cx.set_global(DragSource {
        key: Some(key.into()),
        cancelled_release: false,
        epoch,
    });
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
    let stopped = cx.stop_active_drag(window);
    let state = cx.default_global::<DragSource>();
    state.key = None;
    state.epoch = state.epoch.wrapping_add(1);
    state.cancelled_release |= stopped;
    window.refresh();
    stopped
}

pub(super) fn cancel_epoch(cx: &App) -> u64 {
    cx.try_global::<DragSource>().map_or(0, |state| state.epoch)
}

/// A release after Escape belongs to the cancelled drag, not to a click
/// on the source child. The next press starts an ordinary new gesture.
pub(super) fn begin_press(cx: &mut App) {
    let state = cx.default_global::<DragSource>();
    // A new native drag may belong to a control that does not need a
    // source marker. Never lend it the previous gesture's source key.
    state.key = None;
    state.cancelled_release = false;
}

pub(super) fn discard_release(cx: &mut App) -> bool {
    std::mem::take(&mut cx.default_global::<DragSource>().cancelled_release)
}

/// Opacity of the place an element was lifted from.
pub(super) const SOURCE_OPACITY: f32 = 0.35;

/// Lifted look of a dragged copy: raised above the content with a shadow.
pub(super) fn lifted(element: Div, cx: &App) -> Div {
    let theme = cx.theme();
    element
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .shadow_lg()
        .opacity(0.95)
}
