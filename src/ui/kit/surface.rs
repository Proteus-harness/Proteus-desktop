//! Filled panels alternate the two theme surfaces by actual paint nesting.
//! Header highlights have the same rounded contour whether their disclosure
//! is open or closed.

use std::cell::Cell;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

thread_local! {
    static DEPTH: Cell<usize> = const { Cell::new(0) };
}

pub(crate) fn surface(content: impl IntoElement, cx: &App) -> AnyElement {
    Surface {
        child: content.into_any_element(),
        radius: cx.theme().radius_lg,
        fills: [cx.theme().group_box, cx.theme().accordion],
    }
    .into_any_element()
}

/// Inherits its hover target from the nearest named group, without changing
/// that target's layout, text color, focus, or mouse handling.
pub(crate) fn surface_highlight(group: &'static str, cx: &App) -> Div {
    surface_paint(
        group,
        transparent_black(),
        cx.theme().panel_header_hover,
        px(1.),
        cx,
    )
}

/// The header keeps a symmetric contour in every disclosure state.
pub(crate) fn surface_header(group: &'static str, cx: &App) -> Div {
    surface_paint(
        group,
        transparent_black(),
        cx.theme().panel_header_hover,
        px(0.),
        cx,
    )
}

fn surface_paint(group: &'static str, base: Hsla, hover: Hsla, edge: Pixels, cx: &App) -> Div {
    let radius = cx.theme().radius_lg;
    div()
        .absolute()
        .inset_0()
        .text_color(base)
        .group_hover(group, move |style| style.text_color(hover))
        .child(
            canvas(
                |_, _, _| (),
                move |bounds, (), window, _| {
                    let color = window.text_style().color;
                    if color.a == 0. {
                        return;
                    }
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        let quad = fill(bounds, color)
                            .corner_radii(
                                Corners::all(radius).clamp_radii_for_quad_size(bounds.size),
                            )
                            .border_widths(edge);
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
    fills: [Hsla; 2],
}

/// Count only painted panels. Layout wrappers do not enter this scope, and
/// leaving a child restores the parent's depth before painting its siblings.
fn with_frame<R>(paint: impl FnOnce(usize) -> R) -> R {
    let depth = DEPTH.get();
    DEPTH.set(depth + 1);
    let result = paint(depth);
    DEPTH.set(depth);
    result
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
        with_frame(|depth| {
            window
                .paint_quad(fill(bounds, self.fills[depth % 2]).corner_radii(
                    Corners::all(self.radius).clamp_radii_for_quad_size(bounds.size),
                ));
            self.child.paint(window, cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn nested_panels_alternate_and_siblings_restore_their_parent_scope() {
        assert_eq!(DEPTH.get(), 0);
        let mut fills = Vec::new();
        with_frame(|depth| {
            fills.push(depth % 2);
            with_frame(|depth| {
                fills.push(depth % 2);
                with_frame(|depth| fills.push(depth % 2));
            });
            with_frame(|depth| fills.push(depth % 2));
        });
        with_frame(|depth| fills.push(depth % 2));
        assert_eq!(fills, [0, 1, 0, 1, 0]);
        assert_eq!(DEPTH.get(), 0);
    }
}
