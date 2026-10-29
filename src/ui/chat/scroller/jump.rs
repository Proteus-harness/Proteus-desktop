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
    running: bool,
    hovered: bool,
}

impl JumpButton {
    pub(super) fn new(scroller: WeakEntity<ChatScroller>) -> Self {
        Self {
            scroller,
            running: false,
            hovered: false,
        }
    }

    pub(super) fn set_running(&mut self, running: bool, cx: &mut Context<Self>) {
        if self.running != running {
            self.running = running;
            cx.notify();
        }
    }
}

impl Render for JumpButton {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scroller = self.scroller.clone();
        let icon = if self.running && !self.hovered {
            motion::wave_dots(window, cx).into_any_element()
        } else {
            Icon::new(IconName::ArrowDown).small().into_any_element()
        };
        let button = kit::button("jump-latest")
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
            .flex_none()
            .on_hover(cx.listener(|this, hovered, _, cx| {
                if this.hovered != *hovered {
                    this.hovered = *hovered;
                    cx.notify();
                }
            }))
            .child(button)
    }
}
