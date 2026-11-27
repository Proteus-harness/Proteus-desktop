//! The side panel between the rail and the tabs: one panel at a time,
//! opened from the rail, as wide as its edge is dragged; a double click on
//! the edge gives back the default width. The panel slides within a
//! fixed-width slot.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::preferences::{SIDEBAR_WIDTH, SidebarPreferences};
use crate::ui::workspace::Workspace;
use crate::ui::{divider, drag, motion, prefs};

/// Payload of the side panel's edge while it is dragged.
#[derive(Clone)]
pub(in crate::ui) struct SidebarDrag;

impl Render for SidebarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Workspace {
    /// Chat panels and Settings occupy the same slot beside the rail and
    /// share both its transient drag and its persisted logical width.
    pub(in crate::ui) fn sidebar_width(&self, cx: &App) -> Rems {
        let dragged = cx.has_active_drag().then_some(self.sidebar_drag).flatten();
        prefs::scaled(dragged.unwrap_or(prefs::visual(cx).sidebar.width))
    }

    pub(in crate::ui) fn move_sidebar_resize(&mut self, offset: Pixels, cx: &mut Context<Self>) {
        let width = offset.as_f32() / prefs::scale(cx);
        self.sidebar_drag = Some(width.clamp(*SIDEBAR_WIDTH.start(), *SIDEBAR_WIDTH.end()));
        cx.notify();
    }

    pub(in crate::ui) fn reset_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        self.sidebar_drag = None;
        prefs::update_visual(cx, |prefs| {
            prefs.sidebar.width = SidebarPreferences::default().width
        });
    }

    pub(in crate::ui) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let shown = self.shown_panel(cx);
        let open = shown.is_some();
        let slides = motion::moves(cx);
        let duration = if slides { motion::BASE } else { motion::FAST };
        let progress = motion::blend("sidebar".into(), open, duration, window, cx);
        let panel = match shown {
            Some(panel) => {
                self.sidebar_last = Some(panel.name);
                panel
            }
            None if progress > 0. => super::info(self.sidebar_last?, cx)?,
            None => {
                self.sidebar_last = None;
                return None;
            }
        };
        let width = self.sidebar_width(cx);
        let content = motion::switch_in(
            div()
                .size_full()
                .child(self.render_panel(&panel, window, cx)),
            ElementId::Name(format!("sidebar-panel-{}", panel.name).into()),
        );
        let name = SharedString::from("sidebar-edge");
        let header = prefs::scaled(crate::ui::tabs::BAR_HEIGHT).to_pixels(cx.theme().font_size);
        let seam = divider::ResizeSeam::vertical(header + cx.theme().radius_lg, header).quiet();
        let grip = seam
            .grip(name.clone(), cx)
            .on_drag(SidebarDrag, move |drag, _, _, cx| {
                drag::start(name.clone(), cx);
                cx.new(|_| drag.clone())
            })
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.reset_sidebar_resize(cx);
                }
            }));
        let theme = cx.theme();
        let body = div()
            .absolute()
            .left(if slides {
                width * (progress - 1.)
            } else {
                rems(0.)
            })
            .top_0()
            .h_full()
            .w(width)
            .pr(prefs::scaled(divider::WIDTH))
            .overflow_hidden()
            .opacity(if slides { 1. } else { progress })
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(content);
        // The transcript changes width once at each toggle. Moving or
        // fading the body paints the transition without resizing its rows.
        let slot = div()
            .relative()
            .flex_none()
            .h_full()
            .w(width)
            .overflow_hidden()
            .child(body)
            .when(!open, |slot| {
                slot.capture_any_mouse_down(|_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
            })
            .on_drag_move::<SidebarDrag>(cx.listener(
                |this, event: &DragMoveEvent<SidebarDrag>, _, cx| {
                    let offset = event.event.position.x - event.bounds.left();
                    this.move_sidebar_resize(offset, cx);
                },
            ))
            .children(
                (open && progress == 1.)
                    .then(|| seam.render(grip, cx).absolute().right_0().top_0()),
            );
        Some(slot.into_any_element())
    }

    /// Stores the width the side panel's edge was released at.
    pub(in crate::ui) fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        if let Some(width) = self.sidebar_drag.take() {
            prefs::update_visual(cx, |prefs| prefs.sidebar.width = width);
        }
    }
}
