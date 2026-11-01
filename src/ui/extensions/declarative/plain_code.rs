// Адаптировано из Zed: crates/markdown/src/markdown.rs (plain code text),
// crates/agent_ui/src/conversation_view/thread_view.rs (bounded disclosure).
//! Prose fences share tool output's selectable, naturally wrapping body.
//! The existing fence shell owns the only surface and full-source copy button.

use super::{code_block, code_geometry::Width, fields::Code, view::DeclarativeView};
use crate::ui::kit::Pressable as _;
use crate::ui::{fade, format, kit, motion};
use gpui_kit::base::SelectableText;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const PEEK_LINES: usize = 12;
const PEEK_BYTES: usize = 2_000;
const FULL_LINES: usize = 400;
const FULL_BYTES: usize = 40_000;

pub(super) struct Plain {
    source: SharedString,
    open: bool,
}

impl Plain {
    pub(super) fn new(source: &str) -> Self {
        Self {
            source: source.to_owned().into(),
            open: false,
        }
    }

    pub(super) fn set_source(&mut self, source: &str) {
        if self.source.as_ref() != source {
            self.source = source.to_owned().into();
        }
    }

    pub(super) fn render(
        &self,
        id: &str,
        language: &str,
        cx: &Context<DeclarativeView>,
    ) -> AnyElement {
        let (prefix, long) = bounded_prefix(&self.source, PEEK_LINES, PEEK_BYTES);
        let preview: SharedString = prefix.trim_end().to_owned().into();
        let key: SharedString = format!("plain-code-{}-{id}", cx.entity_id()).into();
        // At rest a collapsed fence does not build, shape, or copy its tail.
        // During closing only the bounded full endpoint stays mounted.
        let expanded = long && (self.open || motion::moving(&key, self.open, cx));
        let shown: SharedString = if expanded {
            format::clip_block(&self.source, FULL_LINES, FULL_BYTES).into()
        } else {
            preview.clone()
        };
        let body = |text: SharedString| {
            // Reuse the tool body's typography without a second frame,
            // padding, editor, wheel handler, or private scroll container.
            kit::code_surface(cx)
                .p_0()
                .min_w_0()
                .text_size(cx.theme().mono_font_size)
                .line_height(relative(1.5))
                .child(SelectableText::new(
                    SharedString::from(format!("plain-code-text-{id}")),
                    text,
                ))
        };
        let disclosure = long.then(|| {
            let toggle_id = id.to_owned();
            let toggle_key = key.clone();
            h_flex()
                .id(ElementId::Name(key.clone()))
                .w_full()
                .flex_none()
                .gap_1()
                .text_xs()
                .disclosure(cx)
                .child(motion::chevron(
                    ElementId::Name(key.clone()),
                    self.open,
                    cx.theme().muted_foreground,
                ))
                .child(fade::faded(if self.open {
                    "Свернуть"
                } else {
                    "Показать больше"
                }))
                .on_click(cx.listener(move |view, _, _, cx| {
                    cx.stop_propagation();
                    if let Some(Code::Plain(plain)) = view.fields.codes.get_mut(&toggle_id) {
                        cx.emit(super::view::WillResize);
                        plain.open = !plain.open;
                        motion::toggled(toggle_key.clone(), plain.open, cx);
                        cx.notify();
                    }
                }))
                .into_any_element()
        });
        let content = if long {
            let preview = div()
                .relative()
                .w_full()
                .min_w_0()
                .overflow_hidden()
                .child(body(preview))
                .child(
                    fade::bottom(cx.theme().muted, crate::ui::prefs::scaled(56.))
                        .max_h(relative(1.)),
                );
            if expanded {
                motion::preview_reveal(key.clone(), self.open, cx, preview, body(shown.clone()))
            } else {
                preview.into_any_element()
            }
        } else {
            body(shown.clone()).into_any_element()
        };
        // Cloning SharedString shares the immutable source. Copy materializes
        // nothing per frame and always includes bytes outside either preview.
        let copy_source = self.source.clone();
        code_block::shell(
            SharedString::from(format!("code-copy-{id}")),
            language,
            move |_, _| copy_source.clone(),
            None,
            content,
            cx,
        )
        // Measuring a closed fence must also avoid shaping the hidden source.
        .fit(Width::Source(shown))
        .when_some(disclosure, |shell, disclosure| {
            shell.disclosure(disclosure, "Показать больше")
        })
        .into_any_element()
    }
}

/// Return a borrowed UTF-8 prefix without scanning even one overlong hidden
/// line. A final LF terminates its row and does not create another display row.
fn bounded_prefix(text: &str, max_lines: usize, max_bytes: usize) -> (&str, bool) {
    let mut lines = 1;
    for (i, byte) in text.bytes().take(max_bytes.saturating_add(1)).enumerate() {
        if i == max_bytes {
            return (&text[..text.floor_char_boundary(i)], true);
        }
        if byte == b'\n' {
            if lines == max_lines && i + 1 < text.len() {
                return (&text[..i], true);
            }
            lines += 1;
        }
    }
    (text, false)
}

#[cfg(test)]
mod tests {
    use super::{PEEK_BYTES, PEEK_LINES, Plain, bounded_prefix};

    #[test]
    fn preview_omits_only_the_final_cursor_row_and_preserves_full_copy_source() {
        let source = ["row"; PEEK_LINES].join("\n") + "\n";
        assert_eq!(
            bounded_prefix(&source, PEEK_LINES, PEEK_BYTES),
            (source.as_str(), false)
        );
        let long = source.clone() + "last row\n";
        let (prefix, clipped) = bounded_prefix(&long, PEEK_LINES, PEEK_BYTES);
        assert!(clipped);
        assert_eq!(prefix.lines().count(), PEEK_LINES);
        let plain = Plain::new(&long);
        assert_eq!(plain.source.as_ref(), long);
    }

    #[test]
    fn preview_bounds_multibyte_and_very_long_single_lines() {
        for source in ["я".repeat(1_000_000), "🚀".repeat(500_000)] {
            let (prefix, clipped) = bounded_prefix(&source, PEEK_LINES, 1_999);
            assert!(clipped);
            assert!(prefix.len() <= 1_999);
            assert!(source.is_char_boundary(prefix.len()));
            assert_eq!(prefix.as_ptr(), source.as_ptr());
        }
        assert!(!bounded_prefix(&"a".repeat(PEEK_BYTES), PEEK_LINES, PEEK_BYTES).1);
        assert!(bounded_prefix(&"a".repeat(PEEK_BYTES + 1), PEEK_LINES, PEEK_BYTES).1);
    }

    #[test]
    fn retained_snapshot_edits_keep_disclosure_and_complete_source() {
        let mut plain = Plain::new("before");
        plain.open = true;
        plain.set_source("after\n\n");
        assert!(plain.open);
        assert_eq!(plain.source.as_ref(), "after\n\n");
        // SharedString may inline short values; use a heap-backed snapshot
        // when checking that an unchanged update keeps its shared allocation.
        let long = "after".repeat(100) + "\n\n";
        plain.set_source(&long);
        let snapshot = plain.source.clone();
        plain.set_source(&long);
        assert!(plain.open);
        assert_eq!(plain.source.as_ref(), long);
        assert_eq!(plain.source.as_ptr(), snapshot.as_ptr());
    }
}
