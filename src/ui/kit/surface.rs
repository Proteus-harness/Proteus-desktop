//! Header highlights reuse their containing frame's quad. GPUI's overflow
//! mask is rectangular, so rounding a shorter header independently cannot
//! follow a large card's corners. Clip the frame-sized fill to the header.

use std::cell::RefCell;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

thread_local! {
    static FRAMES: RefCell<Vec<(Bounds<Pixels>, Pixels)>> = const { RefCell::new(Vec::new()) };
}

pub(crate) fn surface(content: impl IntoElement, cx: &App) -> AnyElement {
    Surface {
        child: content.into_any_element(),
        radius: cx.theme().radius_lg,
    }
    .into_any_element()
}

/// Inherits its hover target from the nearest named group, without changing
/// that target's layout, text color, focus, or mouse handling.
pub(crate) fn surface_highlight(group: &'static str, cx: &App) -> Div {
    let hover = cx.theme().list_hover;
    let radius = cx.theme().radius_lg;
    div()
        .absolute()
        .inset_0()
        .text_color(transparent_black())
        .group_hover(group, move |style| style.text_color(hover))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    let color = window.text_style().color;
                    if color.a == 0. {
                        return;
                    }
                    let frame = FRAMES.with_borrow(|frames| frames.last().copied());
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        let quad = if let Some((frame, radius)) = frame {
                            fill(frame, color)
                                .corner_radii(
                                    Corners::all(radius).clamp_radii_for_quad_size(frame.size),
                                )
                                .border_widths(px(1.))
                        } else {
                            fill(bounds, color).corner_radii(radius)
                        };
                        window.paint_quad(quad);
                    });
                },
            )
            .size_full(),
        )
}

struct Surface {
    child: AnyElement,
    radius: Pixels,
}

impl IntoElement for Surface {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for Surface {
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
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        FRAMES.with_borrow_mut(|frames| frames.push((bounds, self.radius)));
        self.child.paint(window, cx);
        FRAMES.with_borrow_mut(|frames| {
            frames.pop();
        });
    }
}
