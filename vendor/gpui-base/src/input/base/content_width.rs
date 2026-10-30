//! Viewport-independent, capped measurement for content-sized readonly inputs.

use gpui::{Font, Pixels, TextRun, Window, px};

use super::{
    InputBaseState, InputModeKind, RopeExt as _,
    element::{RIGHT_MARGIN, TextElement},
    tab_stops::{expanded_width, tab_stop_width},
};

#[derive(PartialEq)]
struct WidthKey {
    revision: u64,
    font: Font,
    text_size: Pixels,
    tab_size: usize,
}

#[derive(PartialEq)]
struct GutterKey {
    rows: usize,
    minimum_digits: usize,
    line_numbers: bool,
    folding: bool,
}

pub(super) struct ContentWidthCache {
    key: WidthKey,
    widest: Pixels,
    next_row: usize,
    complete: bool,
    gutter: Option<(GutterKey, Pixels)>,
}

impl ContentWidthCache {
    fn new(key: WidthKey) -> Self {
        Self {
            key,
            widest: px(0.),
            next_row: 0,
            complete: false,
            gutter: None,
        }
    }
}

impl<M: InputModeKind> InputBaseState<M> {
    /// Measure the unwrapped source width, capped at `max_width`.
    ///
    /// Includes the current line-number/folding gutter and the editor's small
    /// trailing text margin, but excludes input frame padding. `font` and
    /// `text_size` must match the rendered input. Soft wrapping and the current
    /// viewport do not affect this measurement.
    ///
    /// Rows are shaped only until the cap is reached. The cache retains a key,
    /// widths and a row cursor, without another source copy or shaped-line list.
    /// Unchanged text at the same or a smaller cap needs no further shaping;
    /// increasing the cap resumes an unfinished scan. A complete measurement
    /// survives resizing. This measures source rows, including folded content.
    pub fn unwrapped_content_width(
        &self,
        font: Font,
        text_size: Pixels,
        max_width: Pixels,
        window: &mut Window,
    ) -> Pixels {
        let max_width = max_width.max(px(0.));
        if max_width == px(0.) {
            return max_width;
        }

        let key = WidthKey {
            revision: self.document_revision,
            font,
            text_size,
            tab_size: self.mode.tab_size().tab_size,
        };
        let mut cache = match self.unwrapped_content_width.take() {
            Some(mut cache) => {
                if cache.key != key {
                    *cache = ContentWidthCache::new(key);
                }
                cache
            }
            None => Box::new(ContentWidthCache::new(key)),
        };

        let gutter_key = GutterKey {
            rows: self.presentation_buffer_line_count(),
            minimum_digits: self.mode.min_line_number_digits(),
            line_numbers: self.mode.line_number(),
            folding: self.mode.is_folding(),
        };
        let gutter = match cache.gutter.as_ref() {
            Some((key, width)) if *key == gutter_key => *width,
            _ => {
                let (width, _) = TextElement::layout_line_numbers_for_font(
                    self,
                    cache.key.font.clone(),
                    text_size,
                    window,
                );
                cache.gutter = Some((gutter_key, width));
                width
            }
        };
        let text_cap = (max_width - gutter - RIGHT_MARGIN).max(px(0.));

        if !cache.complete && cache.widest < text_cap {
            let stop = tab_stop_width(
                window.text_system(),
                &cache.key.font,
                text_size,
                cache.key.tab_size,
            );
            let rows = self.presentation_buffer_line_count();
            while cache.next_row < rows && cache.widest < text_cap {
                let source: gpui::SharedString =
                    self.text.slice_line(cache.next_row).to_string().into();
                let shaped = window.text_system().shape_line(
                    source.clone(),
                    text_size,
                    &[TextRun {
                        len: source.len(),
                        font: cache.key.font.clone(),
                        color: gpui::black(),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    }],
                    None,
                );
                cache.widest = cache.widest.max(expanded_width(&shaped, &source, stop));
                cache.next_row += 1;
            }
            cache.complete = cache.next_row >= rows;
        }

        let width = (cache.widest + gutter + RIGHT_MARGIN).min(max_width);
        self.unwrapped_content_width.set(Some(cache));
        width
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext, size};

    fn assert_close(actual: Pixels, expected: Pixels) {
        assert!(
            f32::from((actual - expected).abs()) < 0.01,
            "expected {expected:?}, got {actual:?}"
        );
    }

    fn cached_rows<M: InputModeKind>(state: &InputBaseState<M>) -> (usize, bool) {
        let cache = state.unwrapped_content_width.take().unwrap();
        let result = (cache.next_row, cache.complete);
        state.unwrapped_content_width.set(Some(cache));
        result
    }

    #[gpui::test]
    fn content_width_uses_widest_shaped_row_not_byte_length(cx: &mut TestAppContext) {
        cx.update(crate::init);
        cx.open_window(size(px(400.), px(100.)), |window, cx| {
            let font = window.text_style().font();
            cx.new(|cx| {
                let short_bytes = "x".repeat(80);
                let long_bytes = "é".repeat(60);
                assert!(short_bytes.len() < long_bytes.len());
                let mut state = crate::input::EditorState::new(window, cx)
                    .line_number(false)
                    .folding(false)
                    .default_value(short_bytes.clone());
                let expected =
                    state.unwrapped_content_width(font.clone(), px(14.), px(10_000.), window);
                state.set_value(long_bytes.clone(), window, cx);
                let narrower =
                    state.unwrapped_content_width(font.clone(), px(14.), px(10_000.), window);
                assert!(expected > narrower);
                state.set_value(format!("{long_bytes}\n{short_bytes}"), window, cx);
                assert_close(
                    state.unwrapped_content_width(font, px(14.), px(10_000.), window),
                    expected,
                );
                state
            });
            gpui::EmptyView
        });
    }

    #[gpui::test]
    fn content_width_cache_handles_caps_resize_revision_and_font_size(cx: &mut TestAppContext) {
        cx.update(crate::init);
        cx.open_window(size(px(400.), px(100.)), |window, cx| {
            let font = window.text_style().font();
            cx.new(|cx| {
                let mut state = crate::input::EditorState::new(window, cx)
                    .line_number(false)
                    .folding(false)
                    .default_value(
                        "xxxxxxxxxxxxxxxxxxxx\nx\nxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                    );
                assert_eq!(
                    state.unwrapped_content_width(font.clone(), px(14.), px(40.), window),
                    px(40.)
                );
                assert_eq!(cached_rows(&state), (1, false));
                for cap in [px(30.), px(40.)] {
                    assert_eq!(
                        state.unwrapped_content_width(font.clone(), px(14.), cap, window),
                        cap
                    );
                    assert_eq!(cached_rows(&state), (1, false));
                }
                let width =
                    state.unwrapped_content_width(font.clone(), px(14.), px(10_000.), window);
                assert_eq!(cached_rows(&state), (3, true));
                assert_close(
                    state.unwrapped_content_width(font.clone(), px(14.), px(20_000.), window),
                    width,
                );
                assert_eq!(cached_rows(&state), (3, true));
                let larger =
                    state.unwrapped_content_width(font.clone(), px(28.), px(10_000.), window);
                assert!(larger > width);
                state.set_value("x", window, cx);
                assert!(state.unwrapped_content_width(font, px(28.), px(10_000.), window) < width);
                assert_eq!(cached_rows(&state), (1, true));
                state
            });
            gpui::EmptyView
        });
    }

    #[gpui::test]
    fn content_width_uses_editor_tab_stops(cx: &mut TestAppContext) {
        cx.update(crate::init);
        cx.open_window(size(px(400.), px(100.)), |window, cx| {
            let font = window.text_style().font();
            let text_size = px(14.);
            let stop = tab_stop_width(window.text_system(), &font, text_size, 4);
            cx.new(|cx| {
                let mut state = crate::input::EditorState::new(window, cx)
                    .line_number(false)
                    .folding(false)
                    .tab_size(crate::input::TabSize {
                        tab_size: 4,
                        hard_tabs: true,
                    })
                    .default_value("\t\t");
                let gutter = TextElement::layout_line_numbers_for_font(
                    &state,
                    font.clone(),
                    text_size,
                    window,
                )
                .0;
                assert_close(
                    state.unwrapped_content_width(font.clone(), text_size, px(10_000.), window),
                    stop * 2. + gutter + RIGHT_MARGIN,
                );
                state.set_tab_size(
                    crate::input::TabSize {
                        tab_size: 8,
                        hard_tabs: true,
                    },
                    cx,
                );
                assert_close(
                    state.unwrapped_content_width(font, text_size, px(10_000.), window),
                    stop * 4. + gutter + RIGHT_MARGIN,
                );
                state
            });
            gpui::EmptyView
        });
    }

    #[gpui::test]
    fn content_width_tracks_gutter_and_final_row_without_source_edits(cx: &mut TestAppContext) {
        cx.update(crate::init);
        cx.open_window(size(px(400.), px(100.)), |window, cx| {
            let font = window.text_style().font();
            cx.new(|cx| {
                let mut state = crate::input::EditorState::new(window, cx)
                    .min_line_number_digits(1)
                    .folding(false)
                    .default_value("x\n".repeat(9));
                state.set_readonly(true, cx);
                let measured = |state: &crate::input::EditorState, window: &mut Window| {
                    state.unwrapped_content_width(font.clone(), px(14.), px(10_000.), window)
                };
                let ten = measured(&state, window);
                state.set_show_trailing_empty_line(false, cx);
                let nine = measured(&state, window);
                assert!(nine < ten, "hiding row 10 must narrow the gutter");
                state.set_min_line_number_digits(3, cx);
                assert!(measured(&state, window) > ten);
                state.set_line_number(false, window, cx);
                assert!(measured(&state, window) < nine);
                assert_eq!(
                    state.unwrapped_content_width(font, px(14.), px(5.), window),
                    px(5.)
                );
                state
            });
            gpui::EmptyView
        });
    }
}
