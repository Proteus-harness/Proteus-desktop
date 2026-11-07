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

/// One geometry for a resize seam's quiet line, hover/held mark and hit strip.
/// Rounded neighbours can inset the paint without shrinking the drag target.
#[derive(Clone, Copy)]
pub(super) struct ResizeSeam {
    axis: Axis,
    visible_start: Pixels,
    hit_start: Pixels,
}

impl ResizeSeam {
    pub(super) fn plain(axis: Axis) -> Self {
        Self {
            axis,
            visible_start: px(0.),
            hit_start: px(0.),
        }
    }

    /// Vertical seam offsets in already scaled pixels, from the layout top.
    /// The mark and hairline start together; the grip may start above them.
    pub(super) fn vertical(visible_start: Pixels, hit_start: Pixels) -> Self {
        Self {
            axis: Axis::Row,
            visible_start,
            hit_start,
        }
    }

    /// Paint a functional hairline, then its deferred interactive grip.
    pub(super) fn render(self, grip: Stateful<Div>, cx: &App) -> Div {
        let hairline = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(cx.theme().border.opacity(0.35));
        let hairline = match self.axis {
            Axis::Row => hairline.top(self.visible_start),
            Axis::Column => hairline.top_0(),
        };
        line(self.axis, cx)
            .relative()
            .child(hairline)
            .child(deferred(grip))
    }

    /// Build the seven-pixel hit strip without padding away its upper part.
    pub(super) fn grip(self, name: SharedString, cx: &App) -> Stateful<Div> {
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
        match self.axis {
            Axis::Row => grip
                .top(self.hit_start)
                .bottom_0()
                .left(-reach)
                .right(-reach)
                .cursor(CursorStyle::ResizeLeftRight)
                .child(
                    mark.absolute()
                        .top(self.visible_start - self.hit_start)
                        .bottom_0()
                        .left(reach)
                        .w(prefs::scaled(MARK)),
                ),
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
}

/// A rectangular resize seam without a visual or hit inset.
pub(super) fn resizable(axis: Axis, grip: Stateful<Div>, cx: &App) -> Div {
    ResizeSeam::plain(axis).render(grip, cx)
}

/// The strip over a divider wider than its hairline that takes the
/// pointer, named `name` as a drag source; marked by a line while hovered
/// or dragged.
pub(super) fn grip(name: SharedString, axis: Axis, cx: &App) -> Stateful<Div> {
    ResizeSeam::plain(axis).grip(name, cx)
}
