//! Width and overflow presentation for readonly fences. The editor keeps its
//! own scroll system; these decorations never register a pointer hitbox.
use gpui_kit::base::{ScrollbarHandle as _, input::EditorScrollbarContext};
use gpui_kit::component::{ActiveTheme as _, input::EditorState, scroll::Scrollbar};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub(super) enum Width {
    Editor(Entity<EditorState>, bool),
    Source(SharedString),
}

impl Width {
    pub(super) fn measure(&self, id: &ElementId, window: &mut Window, cx: &mut App) -> Pixels {
        let font = Font {
            family: cx.theme().mono_font_family.clone(),
            ..Font::default()
        };
        let size = cx.theme().mono_font_size;
        let cap = window.viewport_size().width;
        match self {
            Self::Editor(state, numbered) => {
                // xsmall Editor has 4 px input insets on both sides; numbered
                // fences add the same 6 px inset as their rendered Editor.
                let insets = px(8.)
                    + if *numbered {
                        rems(0.375).to_pixels(window.rem_size())
                    } else {
                        px(0.)
                    };
                state
                    .read(cx)
                    .unwrapped_content_width(font, size, cap, window)
                    + insets
            }
            Self::Source(source) => {
                let cache = window.use_keyed_state((id.clone(), "source-width"), cx, |_, _| {
                    SourceWidth::default()
                });
                cache.update(cx, |cache, _| {
                    cache.measure(source, font, size, cap, window)
                })
            }
        }
    }
}

/// Fallback source already owns a SharedString. Share that snapshot, keeping
/// only the measured scalar; never allocate a second source for measurement.
#[derive(Default)]
struct SourceWidth {
    source: SharedString,
    font: Font,
    size: Pixels,
    cap: Pixels,
    width: Pixels,
    complete: bool,
}

impl SourceWidth {
    fn measure(
        &mut self,
        source: &SharedString,
        font: Font,
        size: Pixels,
        cap: Pixels,
        window: &mut Window,
    ) -> Pixels {
        if self.source == *source
            && self.font == font
            && self.size == size
            && (self.complete || cap <= self.cap)
        {
            return self.width.min(cap);
        }
        let mut width = px(0.);
        let mut complete = true;
        for line in source.split_terminator('\n') {
            // Match the plain SelectableText shaper used by this fallback.
            let line: SharedString = line.trim_end_matches('\r').to_owned().into();
            let run = TextRun {
                len: line.len(),
                font: font.clone(),
                color: black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            width = width.max(
                window
                    .text_system()
                    .shape_line(line, size, &[run], None)
                    .width(),
            );
            if width >= cap {
                complete = false;
                break;
            }
        }
        *self = Self {
            source: source.clone(),
            font,
            size,
            cap,
            width,
            complete,
        };
        width.min(cap)
    }
}

fn hidden_edges(width: Pixels, content: Pixels, offset: Pixels) -> (bool, bool) {
    let max = (content - width).max(px(0.));
    (
        max > px(1.) && offset < px(-1.),
        max > px(1.) && offset > px(1.) - max,
    )
}

fn fade(leading: bool, color: Hsla) -> Div {
    let (from, to) = if leading {
        (color, color.opacity(0.))
    } else {
        (color.opacity(0.), color)
    };
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .w(rems(1.))
        .when(leading, |this| this.left_0())
        .when(!leading, |this| this.right_0())
        .bg(linear_gradient(
            90.,
            linear_color_stop(from, 0.),
            linear_color_stop(to, 1.),
        ))
}

/// Called by the editor after its caret scroll, so the fades agree with this
/// frame's text, including selection-driven scrolling. Gutter stays outside.
pub(super) fn editor_scrollbar(
    context: EditorScrollbarContext,
    _: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let (left, right) = hidden_edges(
        context.viewport_bounds().size.width,
        context.viewport_bounds().size.width + context.horizontal_scroll_range(),
        context.offset().x,
    );
    div()
        .relative()
        .size_full()
        .when(left, |this| this.child(fade(true, cx.theme().muted)))
        .when(right, |this| this.child(fade(false, cx.theme().muted)))
        .child(
            Scrollbar::new(&context)
                .axis(context.axis())
                .viewport_from_layout(),
        )
        .into_any_element()
}

/// The plain source uses a native ScrollHandle. Read it during prepaint,
/// after layout, rather than introducing a one-frame overflow observer.
pub(super) fn source_fades(handle: ScrollHandle, cx: &App) -> impl IntoElement {
    let color = cx.theme().muted;
    canvas(
        move |bounds, _, _| {
            (
                bounds,
                hidden_edges(
                    bounds.size.width,
                    handle.max_offset().x + bounds.size.width,
                    handle.offset().x,
                ),
            )
        },
        move |_, (bounds, (left, right)), window, _| {
            let width = rems(1.)
                .to_pixels(window.rem_size())
                .min(bounds.size.width / 2.);
            for leading in [true, false] {
                if !(if leading { left } else { right }) {
                    continue;
                }
                let (from, to) = if leading {
                    (color, color.opacity(0.))
                } else {
                    (color.opacity(0.), color)
                };
                let origin = point(
                    if leading {
                        bounds.left()
                    } else {
                        bounds.right() - width
                    },
                    bounds.top(),
                );
                window.paint_quad(fill(
                    Bounds::new(origin, size(width, bounds.size.height)),
                    linear_gradient(90., linear_color_stop(from, 0.), linear_color_stop(to, 1.)),
                ));
            }
        },
    )
    .absolute()
    .inset_0()
}

#[cfg(test)]
mod tests {
    use super::hidden_edges;
    use gpui_kit::px;
    #[test]
    fn edges_follow_real_scroll_range_and_endpoints() {
        assert_eq!(hidden_edges(px(300.), px(300.), px(0.)), (false, false));
        assert_eq!(hidden_edges(px(300.), px(800.), px(0.)), (false, true));
        assert_eq!(hidden_edges(px(300.), px(800.), px(-250.)), (true, true));
        assert_eq!(hidden_edges(px(300.), px(800.), px(-500.)), (true, false));
        assert_eq!(hidden_edges(px(300.), px(300.5), px(0.)), (false, false));
    }
}
