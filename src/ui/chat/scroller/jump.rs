//! Retained jump control. Its hover and animated status never invalidate the
//! chat, remeasure list rows, or prepare tool diffs.

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::*;
use gpui_kit::*;

use super::{ChatScroller, LatestRequested};
use crate::ui::kit::{self, Hint as _};
use crate::ui::motion;

pub(super) struct JumpButton {
    scroller: WeakEntity<ChatScroller>,
    status: JumpStatus,
}

/// Visibility belongs to the retained control too: returning a cached view
/// after visiting the tail must restart its clock even if the run is unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct JumpStatus {
    active: bool,
    visible: bool,
}

impl JumpStatus {
    pub(super) fn new(running: bool, thinking: bool, visible: bool) -> Self {
        Self {
            active: running || thinking,
            visible,
        }
    }

    pub(super) fn wave(self) -> bool {
        self.active && self.visible
    }
}

impl JumpButton {
    pub(super) fn new(scroller: WeakEntity<ChatScroller>, status: JumpStatus) -> Self {
        Self { scroller, status }
    }

    pub(super) fn set_status(&mut self, status: JumpStatus, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            cx.notify();
        }
    }
}

impl Render for JumpButton {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scroller = self.scroller.clone();
        let icon = if self.status.wave() {
            // Native hover styles use the current hitbox, including after a
            // remount with a stationary pointer. No retained hover flag can
            // survive removal and leave an active run stuck on the arrow.
            div()
                .relative()
                .size_4()
                .child(
                    h_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .group_hover("jump-status", |this| this.opacity(0.))
                        .child(motion::wave_dots(window, cx)),
                )
                .child(
                    h_flex()
                        .absolute()
                        .inset_0()
                        .items_center()
                        .justify_center()
                        .opacity(0.)
                        .group_hover("jump-status", |this| this.opacity(1.))
                        .child(Icon::new(IconName::ArrowDown).small()),
                )
                .into_any_element()
        } else {
            Icon::new(IconName::ArrowDown).small().into_any_element()
        };
        let button = kit::icon_button("jump-latest")
            .size_8()
            .secondary()
            .child(
                h_flex()
                    .size_4()
                    .items_center()
                    .justify_center()
                    .child(icon),
            )
            .hint("К последним сообщениям", cx)
            .rounded(cx.theme().radius_full())
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .on_click(move |_, _, cx| {
                let _ = scroller.update(cx, |_, cx| cx.emit(LatestRequested));
            });
        div()
            .id("jump-hover")
            .group("jump-status")
            .flex_none()
            .child(button)
    }
}
