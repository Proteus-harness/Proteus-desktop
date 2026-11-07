//! Shared dividers for sidebars, split panes and data columns.
//! Static seams stay transparent. A resizable seam keeps a faint one-pixel
//! line and a seven-pixel grip, with a brighter hover/drag mark. Every source
//! must call `drag::start` with its own stable key in the drag constructor.

use super::{drag, prefs};
use crate::tabs::Axis;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(super) const WIDTH: f32 = 1.;
const GRIP: f32 = 7.;
const MARK: f32 = WIDTH;

/// An unpainted layout seam; neighbouring surface fills provide separation.
pub(super) fn line(axis: Axis, cx: &App) -> Div {
    match axis {
        Axis::Row => div().w(prefs::scaled(WIDTH)).h_full(),
        Axis::Column => div().h(prefs::scaled(WIDTH)).w_full(),
    }
    .flex_none()
    .bg(cx.theme().transparent)
}

/// A functional, quiet hairline in the layout under `grip`. The grip is drawn
/// after everything else, so the next neighbour does not cover it.
pub(super) fn resizable(axis: Axis, grip: Stateful<Div>, cx: &App) -> Div {
    line(axis, cx)
        .bg(cx.theme().border.opacity(0.35))
        .relative()
        .child(deferred(grip))
}

/// Join a sidebar to the equally colored tab strip above the content.
/// Keep the functional resize seam and its grip below that shared header.
pub(super) fn resizable_below(grip: Stateful<Div>, header: Rems, cx: &App) -> Div {
    line(Axis::Row, cx)
        .relative()
        .child(
            div()
                .absolute()
                .top(header)
                .bottom_0()
                .w_full()
                .bg(cx.theme().border.opacity(0.35)),
        )
        .child(deferred(grip.top(header)))
}

/// The strip over a divider wider than its hairline that takes the
/// pointer, named `name` as a drag source; marked by a line while hovered
/// or dragged.
pub(super) fn grip(name: SharedString, axis: Axis, cx: &App) -> Stateful<Div> {
    let accent = cx.theme().drag_border;
    let held = drag::is_source(&name, cx);
    let reach = px((GRIP - WIDTH) / 2. * prefs::scale(cx));
    let grip = div()
        .id(ElementId::Name(name.clone()))
        .group(name.clone())
        .absolute()
        .flex()
        .justify_center();
    let mark = div()
        .when(held, |mark| mark.bg(accent))
        .group_hover(name, move |style| style.bg(accent));
    match axis {
        Axis::Row => grip
            .top_0()
            .bottom_0()
            .left(-reach)
            .right(-reach)
            .cursor(CursorStyle::ResizeLeftRight)
            .child(mark.w(prefs::scaled(MARK)).h_full()),
        Axis::Column => grip
            .flex_col()
            .left_0()
            .right_0()
            .top(-reach)
            .bottom(-reach)
            .cursor(CursorStyle::ResizeUpDown)
            .child(mark.h(prefs::scaled(MARK)).w_full()),
    }
}
