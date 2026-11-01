// Адаптировано из Zed: crates/markdown/src/markdown.rs (reserved scrollbar
// track) and crates/editor/src/split_editor_view.rs (text/track bounds).
//! Width and overflow presentation for readonly fences. The editor keeps its
//! own scroll system; these decorations never register a pointer hitbox.
use gpui_kit::base::{
    ScrollbarAxis, ScrollbarHandle as _,
    input::{EditorScrollbarContext, EditorScrollbarRenderer},
};
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

/// The rendered track and the reserved strip use one size at every UI scale.
pub(super) fn horizontal_track_height(cx: &App) -> Pixels {
    Scrollbar::width() * crate::ui::prefs::scale(cx)
}

pub(super) fn embedded_editor_height(
    rows: usize,
    line_height: Pixels,
    horizontal_overflow: bool,
    track_height: Pixels,
) -> Pixels {
    // GPUI TextStyle::line_height_in_pixels rounds each row before Editor
    // lays it out. Rounding the aggregate height leaves short fences with a
    // real one-pixel vertical range (13px mono: 2 × round(19.5) = 40).
    line_height.round() * rows as f32
        + if horizontal_overflow {
            track_height
        } else {
            px(0.)
        }
}

fn scrollbar_axis(horizontal: bool, vertical: bool) -> Option<ScrollbarAxis> {
    match (horizontal, vertical) {
        (true, true) => Some(ScrollbarAxis::Both),
        (true, false) => Some(ScrollbarAxis::Horizontal),
        (false, true) => Some(ScrollbarAxis::Vertical),
        (false, false) => None,
    }
}

/// The native editor owns offsets, caret scrolling, hit testing and ranges.
/// Observe its actual horizontal endpoint after layout, without deriving it
/// from content_size (which includes an extra presentation margin).
pub(super) fn embedded_editor_scrollbar(
    owner: WeakEntity<super::view::DeclarativeView>,
    id: String,
    editor_id: EntityId,
) -> EditorScrollbarRenderer {
    std::rc::Rc::new(move |context, window, cx| {
        let horizontal = matches!(
            context.axis(),
            ScrollbarAxis::Horizontal | ScrollbarAxis::Both
        ) && context.horizontal_scroll_range() > px(0.);
        // Do not allocate a deferred callback on ordinary repaint/scroll. The
        // entity identity also fences late prepaint from a replaced Code id.
        let changed = owner.upgrade().is_some_and(|owner| {
            matches!(
                owner.read(cx).fields.codes.get(&id),
                Some(super::fields::Code::Editor { state, horizontal_overflow, .. })
                    if state.entity_id() == editor_id && *horizontal_overflow != horizontal
            )
        });
        if changed {
            let (owner, id) = (owner.clone(), id.clone());
            cx.defer(move |cx| {
                let _ = owner.update(cx, |owner, cx| {
                    let changed = match owner.fields.codes.get_mut(&id) {
                        Some(super::fields::Code::Editor {
                            state,
                            horizontal_overflow,
                            ..
                        }) if state.entity_id() == editor_id
                            && *horizontal_overflow != horizontal =>
                        {
                            *horizontal_overflow = horizontal;
                            true
                        }
                        _ => false,
                    };
                    if changed {
                        // Automatic geometry follows the ordinary rendered
                        // observer/row remeasurement. WillResize marks a user
                        // disclosure gesture and would stop following the chat.
                        cx.notify();
                    }
                });
            });
        }
        editor_scrollbar(context, window, cx)
    })
}

/// Called after caret scroll, so fades and the thumb share this frame's text.
/// Any horizontal track occupies the reserved bottom strip; the gutter stays
/// outside its viewport, exactly as in the editor's native context.
fn editor_scrollbar(context: EditorScrollbarContext, _: &mut Window, cx: &mut App) -> AnyElement {
    let horizontal = matches!(
        context.axis(),
        ScrollbarAxis::Horizontal | ScrollbarAxis::Both
    ) && context.horizontal_scroll_range() > px(0.);
    let vertical = matches!(
        context.axis(),
        ScrollbarAxis::Vertical | ScrollbarAxis::Both
    ) && context.content_size().height > context.viewport_bounds().size.height;
    let axis = scrollbar_axis(horizontal, vertical);
    let (left, right) = hidden_edges(
        context.viewport_bounds().size.width,
        context.viewport_bounds().size.width + context.horizontal_scroll_range(),
        context.offset().x,
    );
    let track_height = horizontal_track_height(cx);
    div()
        .relative()
        .size_full()
        .when(left, |this| this.child(fade(true, cx.theme().muted)))
        .when(right, |this| this.child(fade(false, cx.theme().muted)))
        .when_some(axis, |this, axis| {
            this.child(
                Scrollbar::new(&context)
                    .axis(axis)
                    .viewport_from_layout()
                    .styles(|styles| styles.track(|track| track.width(track_height))),
            )
        })
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
    use super::{embedded_editor_height, hidden_edges};
    use gpui_kit::px;
    #[test]
    fn actual_horizontal_overflow_reserves_one_track_below_all_source_rows() {
        for scale in [0.75, 1., 1.5, 2.] {
            let line = px(19.5 * scale);
            let track = gpui_kit::component::scroll::Scrollbar::width() * scale;
            assert_eq!(
                embedded_editor_height(2, line, false, track),
                line.round() * 2.
            );
            assert_eq!(
                embedded_editor_height(2, line, true, track),
                line.round() * 2. + track
            );
            assert_eq!(
                embedded_editor_height(1, line, true, track),
                line.round() + track
            );
        }
    }

    #[test]
    fn editor_height_rounds_each_row_like_gpui_before_multiplying() {
        for (raw_line, two_rows) in [(19.5, 40.), (23.4, 46.), (23.6, 48.)] {
            assert_eq!(
                embedded_editor_height(2, px(raw_line), false, px(16.)),
                px(two_rows)
            );
            assert_eq!(
                embedded_editor_height(2, px(raw_line), true, px(16.)),
                px(two_rows + 16.)
            );
        }
    }

    #[test]
    fn edges_follow_real_scroll_range_and_endpoints() {
        assert_eq!(hidden_edges(px(300.), px(300.), px(0.)), (false, false));
        assert_eq!(hidden_edges(px(300.), px(800.), px(0.)), (false, true));
        assert_eq!(hidden_edges(px(300.), px(800.), px(-250.)), (true, true));
        assert_eq!(hidden_edges(px(300.), px(800.), px(-500.)), (true, false));
        assert_eq!(hidden_edges(px(300.), px(300.5), px(0.)), (false, false));
    }
}
