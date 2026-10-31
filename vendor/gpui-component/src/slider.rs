// Compact appearance reuses gpui-base Slider/Track/Indicator/Thumb primitives.
// Zed has no settings slider equivalent; interaction remains in the shared base.
use std::sync::Arc;

use crate::{ActiveTheme, AxisExt, StyledExt, ThemeStyled as _};
pub use gpui_base::slider::{SliderEvent, SliderScale, SliderState, SliderValue};
use gpui_base::{Slider as BaseSlider, SliderIndicator, SliderThumb, SliderTrack, spring};

use gpui::{
    App, Axis, Background, Corners, DefiniteLength, Div, ElementId, Entity, EntityId, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, RenderOnce,
    SharedString, StatefulInteractiveElement as _, StyleRefinement, Styled, Window, div,
    prelude::FluentBuilder as _, px, relative,
};

/// Width of the translucent ring that grows outside a hovered thumb.
const THUMB_RING_WIDTH: Pixels = px(3.);
/// Opacity of the fully grown thumb ring.
const THUMB_RING_OPACITY: f32 = 0.5;
/// The animated hover ring of one thumb.
struct ThumbRing {
    interaction: Entity<ThumbInteraction>,
    width: Pixels,
    color: Hsla,
}

/// Pointer state of one thumb, written by its own listeners and read on the
/// next frame to size the ring.
#[derive(Default)]
struct ThumbInteraction {
    hovered: bool,
    pressed: bool,
}

impl ThumbInteraction {
    /// The ring shows while the pointer is over the thumb, and stays while the
    /// thumb is dragged: dragging moves the thumb under the pointer, so hover
    /// alone drops out for a frame on every move and the ring would flicker.
    fn is_active(&self) -> bool {
        self.hovered || self.pressed
    }
}

impl ThumbRing {
    /// Samples the ring for the `start` or end thumb of the slider `id`.
    fn new(
        id: EntityId,
        start: bool,
        color: Hsla,
        compact: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Self {
        let channel = if start { "start" } else { "end" };
        let interaction = window.use_keyed_state(
            ElementId::NamedChild(Arc::new(("slider-thumb-ring", id).into()), channel.into()),
            cx,
            |_, _| ThumbInteraction::default(),
        );
        let progress = spring(
            (("slider-thumb-ring", id), channel),
            if interaction.read(cx).is_active() {
                1.
            } else {
                0.
            },
            cx.theme().motion_tokens().spring_control,
            window,
            cx,
        );

        Self {
            interaction,
            width: if compact {
                window.rem_size() * (3. / 16.)
            } else {
                THUMB_RING_WIDTH
            } * progress,
            color: color.alpha(if compact { 0.2 } else { THUMB_RING_OPACITY } * progress),
        }
    }

    /// Returns a listener that records whether the thumb is being pressed.
    fn press_listener<E>(&self, pressed: bool) -> impl Fn(&E, &mut Window, &mut App) + use<E> {
        let interaction = self.interaction.clone();
        move |_, _, cx| {
            interaction.update(cx, |interaction, cx| {
                if interaction.pressed != pressed {
                    interaction.pressed = pressed;
                    cx.notify();
                }
            });
        }
    }
}

/// A Slider element.
#[derive(IntoElement)]
pub struct Slider {
    state: Entity<SliderState>,
    axis: Axis,
    style: StyleRefinement,
    disabled: bool,
    accessibility_label: Option<SharedString>,
    reverse: bool,
    compact: bool,
}

impl Slider {
    /// Create a new [`Slider`] element bind to the [`SliderState`].
    pub fn new(state: &Entity<SliderState>) -> Self {
        Self {
            axis: Axis::Horizontal,
            state: state.clone(),
            style: StyleRefinement::default(),
            disabled: false,
            accessibility_label: None,
            reverse: false,
            compact: false,
        }
    }

    /// Thin track and outlined thumb, retaining a 24 px pointer target at 100%.
    /// Values, events and keyboard interaction are unchanged.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    /// As a horizontal slider.
    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }

    /// As a vertical slider.
    pub fn vertical(mut self) -> Self {
        self.axis = Axis::Vertical;
        self
    }

    /// Set the disabled state of the slider, default: false
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Sets the label exposed to accessibility clients.
    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// Reverse the filled (highlighted) side of the track, default: false.
    ///
    /// By default the track is filled from the min end to the thumb. With
    /// `reverse`, the fill goes from the thumb to the max end instead — useful
    /// when the slider represents a remaining amount (e.g. time left).
    ///
    /// This only changes the visual fill; values, events and interactions are
    /// unaffected. It applies to single-value sliders and is ignored for
    /// range sliders.
    pub fn reverse(mut self) -> Self {
        self.reverse = true;
        self
    }
}

impl Styled for Slider {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for Slider {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let axis = self.axis;
        let state = self.state.read(cx);
        let is_range = state.value().is_range();
        let percentage = state.percentage();
        let (bar_start, bar_end) = if self.reverse && !is_range {
            // Fill from the thumb to the max end (remaining side).
            (relative(percentage.end), relative(0.))
        } else {
            (relative(percentage.start), relative(1. - percentage.end))
        };
        let rem_size = window.rem_size();
        let compact_hit = rem_size * (24. / 16.);
        let compact_thumb = rem_size * (14. / 16.);
        let compact_track = rem_size * (3. / 16.);
        let compact_offset = (compact_track - compact_hit) / 2.;

        let bar_color = self
            .style
            .background
            .clone()
            .and_then(|bg| bg.color())
            .unwrap_or(cx.theme().tokens.slider_bar.into());
        let thumb_bg: Background = self
            .style
            .text
            .color
            .map(Into::into)
            .unwrap_or_else(|| cx.theme().tokens.slider_thumb.into());
        let corner_radii = self.style.corner_radii.clone();
        // The track is a pill by default, and square when the theme squares its
        // corners. A caller's own corner radii still win.
        let default_radius = cx.theme().radius_full();
        let radius = Corners {
            top_left: corner_radii
                .top_left
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
            top_right: corner_radii
                .top_right
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
            bottom_left: corner_radii
                .bottom_left
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
            bottom_right: corner_radii
                .bottom_right
                .map(|v| v.to_pixels(rem_size))
                .unwrap_or(default_radius),
        };

        let ring_color = cx.theme().ring;
        let entity_id = self.state.entity_id();
        let start_ring =
            is_range.then(|| ThumbRing::new(entity_id, true, ring_color, self.compact, window, cx));
        let end_ring = ThumbRing::new(entity_id, false, ring_color, self.compact, window, cx);

        let thumb = |position: DefiniteLength, start: bool, ring: ThumbRing| {
            SliderThumb::new(&self.state)
                .axis(axis)
                .start(start)
                .disabled(self.disabled)
                .when(!self.disabled, |this| {
                    this.absolute()
                        .when(axis.is_horizontal(), |this| {
                            this.top(if self.compact {
                                compact_offset
                            } else {
                                px(-5.)
                            })
                            .left(position)
                            .ml(if self.compact {
                                -compact_hit / 2.
                            } else {
                                -px(8.)
                            })
                        })
                        .when(axis.is_vertical(), |this| {
                            this.bottom(position)
                                .left(if self.compact {
                                    compact_offset
                                } else {
                                    px(-5.)
                                })
                                .mb(if self.compact {
                                    -compact_hit / 2.
                                } else {
                                    -px(8.)
                                })
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .rounded_full_style(cx)
                        .when(self.compact, |this| this.size(compact_hit))
                        .when(!self.compact, |this| {
                            this.bg(bar_color.opacity(0.5)).size_4().p(px(1.))
                        })
                        .on_hover({
                            let interaction = ring.interaction.clone();
                            move |entered, _, cx| {
                                let entered = *entered;
                                interaction.update(cx, |interaction, cx| {
                                    if interaction.hovered != entered {
                                        interaction.hovered = entered;
                                        cx.notify();
                                    }
                                });
                            }
                        })
                        // The base thumb stops propagation on mouse down to own
                        // the drag, so the press is read in the capture phase.
                        .capture_any_mouse_down(ring.press_listener(true))
                        .capture_any_mouse_up(ring.press_listener(false))
                        .on_mouse_up_out(MouseButton::Left, ring.press_listener(false))
                        .when(!self.compact, |this| this.child(thumb_ring(&ring, cx)))
                        .child(if self.compact {
                            // Only the visual thumb is painted; the transparent
                            // surrounding target retains comfortable dragging.
                            div()
                                .relative()
                                .flex_none()
                                .size(compact_thumb)
                                .rounded_full_style(cx)
                                .border(px(1.))
                                .border_color(
                                    bar_color
                                        .as_solid()
                                        .unwrap_or_else(|| cx.theme().tokens.slider_bar.into()),
                                )
                                .bg(cx.theme().background)
                                .child(thumb_ring(&ring, cx))
                                .into_any_element()
                        } else {
                            div()
                                .flex_shrink_0()
                                .size_full()
                                .rounded_full_style(cx)
                                .bg(thumb_bg)
                                .into_any_element()
                        })
                })
        };

        BaseSlider::new(&self.state)
            .axis(axis)
            .disabled(self.disabled)
            .when_some(self.accessibility_label, |this, label| {
                this.accessibility_label(label)
            })
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .when(axis.is_vertical(), |this| this.h(px(120.)))
            .when(axis.is_horizontal(), |this| this.w_full())
            .refine_style(&self.style)
            .bg(cx.theme().transparent)
            .text_color(cx.theme().foreground)
            .child(
                SliderTrack::new(&self.state)
                    .axis(axis)
                    .disabled(self.disabled)
                    .flex()
                    .when(axis.is_horizontal(), |this| {
                        this.items_center().h_6().w_full()
                    })
                    .when(axis.is_vertical(), |this| {
                        this.justify_center().w_6().h_full()
                    })
                    .flex_shrink_0()
                    .child(
                        SliderIndicator::new(&self.state)
                            .relative()
                            .when(axis.is_horizontal(), |this| {
                                this.w_full()
                                    .when(self.compact, |this| this.h(compact_track))
                                    .when(!self.compact, |this| this.h_1p5())
                            })
                            .when(axis.is_vertical(), |this| {
                                this.h_full()
                                    .when(self.compact, |this| this.w(compact_track))
                                    .when(!self.compact, |this| this.w_1p5())
                            })
                            .bg(bar_color.opacity(0.2))
                            .active(|this| this.bg(bar_color.opacity(0.4)))
                            .corner_radii(radius)
                            .child(
                                div()
                                    .absolute()
                                    .when(axis.is_horizontal(), |this| {
                                        this.h_full().left(bar_start).right(bar_end)
                                    })
                                    .when(axis.is_vertical(), |this| {
                                        this.w_full().bottom(bar_start).top(bar_end)
                                    })
                                    .bg(bar_color)
                                    .rounded_full_style(cx),
                            )
                            .when_some(start_ring, |this, ring| {
                                this.child(thumb(relative(percentage.start), true, ring))
                            })
                            .child(thumb(relative(percentage.end), false, end_ring)),
                    ),
            )
    }
}

/// Hover decoration around the visual thumb rather than its hit target.
fn thumb_ring(ring: &ThumbRing, cx: &App) -> Div {
    div()
        .flex_none()
        .absolute()
        .top(-ring.width)
        .left(-ring.width)
        .right(-ring.width)
        .bottom(-ring.width)
        .rounded_full_style(cx)
        .border(ring.width)
        .border_color(ring.color)
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext as _, Context, Modifiers, Render, TestAppContext, point};

    use super::*;

    struct Harness {
        state: Entity<SliderState>,
        disabled: bool,
        compact: bool,
    }

    impl Render for Harness {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(100.)).h(px(24.)).child(
                Slider::new(&self.state)
                    .disabled(self.disabled)
                    .when(self.compact, Slider::compact),
            )
        }
    }

    fn harness(
        cx: &mut TestAppContext,
        disabled: bool,
        compact: bool,
    ) -> (&mut gpui::VisualTestContext, Entity<SliderState>) {
        cx.update(crate::theme::init);
        let state = cx.new(|_| SliderState::new());
        let result = state.clone();
        let (_, cx) = cx.add_window_view(move |_, _| Harness {
            state,
            disabled,
            compact,
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        (cx, result)
    }

    #[gpui::test]
    fn pointer_updates_the_migrated_state(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, false, false);
        cx.simulate_click(point(px(50.), px(12.)), Modifiers::default());
        cx.update(|_, cx| assert!((state.read(cx).value().end() - 50.).abs() < 1.));
    }

    #[gpui::test]
    fn disabled_slider_is_inert(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, true, false);
        cx.simulate_click(point(px(50.), px(12.)), Modifiers::default());
        cx.update(|_, cx| assert_eq!(state.read(cx).value(), SliderValue::Single(0.)));
    }

    #[gpui::test]
    fn compact_slider_keeps_track_clicks_and_dragging(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, false, true);
        cx.simulate_click(point(px(50.), px(12.)), Modifiers::default());
        cx.update(|window, cx| {
            assert!((state.read(cx).value().end() - 50.).abs() < 1.);
            window.draw(cx).clear(cx);
        });
        // Above the 14 px painted thumb but within its 24 px pointer target.
        cx.simulate_mouse_down(
            point(px(50.), px(2.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(65.), px(2.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_move(
            point(px(80.), px(2.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        cx.simulate_mouse_up(
            point(px(80.), px(2.)),
            MouseButton::Left,
            Modifiers::default(),
        );
        cx.update(|_, cx| assert!((state.read(cx).value().end() - 80.).abs() < 1.));
    }

    #[gpui::test]
    fn compact_disabled_slider_is_inert(cx: &mut TestAppContext) {
        let (cx, state) = harness(cx, true, true);
        cx.simulate_click(point(px(50.), px(12.)), Modifiers::default());
        cx.update(|_, cx| assert_eq!(state.read(cx).value(), SliderValue::Single(0.)));
    }
}
