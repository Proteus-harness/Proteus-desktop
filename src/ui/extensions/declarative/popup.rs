//! Placement and focus of any process extension's badge popup.

use super::SurfaceKind;
use super::badge::DeclarativeBadge;
use super::view::DeclarativeView;
use crate::ui::{motion, prefs};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

impl DeclarativeBadge {
    pub(super) fn toggle_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.popup.is_open() {
            self.close_popup(window, cx);
        } else {
            self.restore = window.focused(cx);
            let popup = super::surface(self.host.clone(), SurfaceKind::Popup, None, window, cx);
            if let Some((root, keys)) = self.last_popup.clone()
                && let Ok(view) = popup.view.clone().downcast::<DeclarativeView>()
            {
                view.update(cx, |view, cx| view.show(root, 0, keys, cx));
            }
            popup.focus.focus(window, cx);
            self.popup.show(popup);
        }
        cx.notify();
    }

    pub(super) fn close_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.popup.current().cloned() else {
            return;
        };
        self.popup.close();
        if let Ok(view) = popup.view.clone().downcast::<DeclarativeView>() {
            self.last_popup = view.read(cx).snapshot().or(self.last_popup.take());
        }
        motion::restore_focus(&popup.focus, self.restore.take(), window, cx);
        cx.notify();
    }

    pub(super) fn render_popup(
        &mut self,
        width: f32,
        height: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let panel = super::super::find(self.host.extension()).and_then(|manifest| {
            self.host
                .badge_panel(super::super::badge_place(manifest, cx), cx)
        });
        let trigger = self.bounds.get();
        let bounds = panel.map_or(trigger, |panel| panel.anchor(trigger));
        let mut frame = self.popup.frame(bounds, window, cx)?;
        if let Some(panel) = panel {
            frame = frame.attached(panel);
        }
        let mut width = prefs::scaled(width).to_pixels(window.rem_size());
        let mut height = prefs::scaled(height).to_pixels(window.rem_size());
        if let Some(panel) = panel {
            // These limits belong to the inner body; reserve the two
            // cross-axis edges of the common outer frame.
            let border = px(2.);
            if panel.placement.is_horizontal() {
                height = height.min((panel.bounds.size.height - border).max(px(0.)));
            } else {
                width = width.min((panel.bounds.size.width - border).max(px(0.)));
            }
        }
        let popup = self.popup.value()?.clone();
        let generation = self.popup.generation();
        let this = cx.entity().downgrade();
        let trigger = self.bounds.clone();
        let surface = v_flex()
            .relative()
            .id("extension-popup")
            .track_focus(&popup.focus)
            .w(width)
            .max_h(height)
            .text_sm()
            .child(popup.view.clone())
            .when(frame.open, |surface| {
                surface
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "tab" {
                            if let Some(popup) = this.popup.current() {
                                crate::ui::kit::cycle_focus(
                                    &popup.focus,
                                    !event.keystroke.modifiers.shift,
                                    window,
                                    cx,
                                );
                            }
                            cx.stop_propagation();
                        } else if event.keystroke.key == "escape" {
                            let action = super::installed(this.host.extension())
                                .and_then(|extension| extension.manifest.surfaces.popup.as_ref())
                                .and_then(|popup| popup.escape_action.clone());
                            if let Some(action) = action {
                                this.view.update(cx, |view, cx| view.act(action, None, cx));
                            } else {
                                this.close_popup(window, cx);
                            }
                            cx.stop_propagation();
                        }
                    }))
                    .on_mouse_down_out(move |event, window, cx| {
                        if !trigger.get().contains(&event.position) {
                            let this = this.clone();
                            window.defer(cx, move |window, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    if this.popup.is_current(generation) {
                                        this.close_popup(window, cx);
                                    }
                                });
                            });
                        }
                    })
            });
        let surface = if frame.open {
            surface
                .focus_trap("extension-popup-trap", &popup.focus)
                .into_any_element()
        } else {
            surface.into_any_element()
        };
        Some(motion::popup(
            surface,
            SharedString::from(format!("badge-popup-{}", cx.entity_id())),
            frame,
            cx,
        ))
    }
}
