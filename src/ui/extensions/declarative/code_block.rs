// Адаптировано из Zed: crates/markdown/src/markdown.rs (code surfaces/copy),
// crates/markdown/src/mermaid.rs (source/preview controls).
//! One native fence shell for code, contributed previews and source fallback.
//! Only the visible body is mounted; the shell never creates an editor or a
//! renderer. Full-document content deliberately bypasses it.

use super::code_geometry::Width;
use crate::ui::kit::{self, Frame as _, Hint as _};
use gpui_kit::base::RoundedFrameCover;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Plain prose fences have no gutter; programming languages keep the
/// existing unwrapped editor presentation. This policy also applies when a
/// retained Code node changes its language without changing its id.
pub(super) fn line_numbers(language: &str, wrap: bool) -> bool {
    !wrap
        && ![
            "",
            "md",
            "markdown",
            "text",
            "plaintext",
            "txt",
            "text/plain",
            "text/markdown",
        ]
        .iter()
        .any(|plain| language.trim().eq_ignore_ascii_case(plain))
}

/// A final LF terminates the preceding source row; it does not add a blank
/// editor cursor row to a readonly preview. Further blank rows stay visible.
pub(super) fn preview_rows(source: &str) -> usize {
    source.split_terminator('\n').count().max(1)
}

/// Shared code presentation for embedded and standalone readonly widgets.
/// Reuses the retained editor while keeping its body and opaque gutter on the
/// same surface as the fence shell.
pub(super) fn readonly_editor(
    state: &Entity<gpui_kit::component::input::EditorState>,
    cx: &App,
) -> gpui_kit::component::input::Editor {
    gpui_kit::component::input::Editor::new(state)
        .xsmall()
        .readonly(true)
        .appearance(false)
        .bordered(false)
        .bg(cx.theme().muted)
}

pub(super) fn shell<F: Fn(&mut Window, &mut App) -> SharedString + 'static>(
    copy_id: impl Into<ElementId>,
    language: &str,
    copy_source: F,
    controls: Option<AnyElement>,
    body: AnyElement,
    _: &App,
) -> Shell<F> {
    Shell {
        copy_id: copy_id.into(),
        language: language.to_owned(),
        copy_source,
        controls,
        body,
        width: None,
    }
}

#[derive(IntoElement)]
pub(super) struct Shell<F: Fn(&mut Window, &mut App) -> SharedString + 'static> {
    copy_id: ElementId,
    language: String,
    copy_source: F,
    controls: Option<AnyElement>,
    body: AnyElement,
    width: Option<Width>,
}

impl<F: Fn(&mut Window, &mut App) -> SharedString + 'static> Shell<F> {
    pub(super) fn fit(mut self, width: Width) -> Self {
        self.width = Some(width);
        self
    }
}

impl<F: Fn(&mut Window, &mut App) -> SharedString + 'static> RenderOnce for Shell<F> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Keep the language and action slots readable even for an empty fence.
        // These are the same text_xs and xsmall icon sizes as the header below.
        let width = self.width.as_ref().map(|width| {
            let font = window.text_style().font();
            let font_size = rems(0.75).to_pixels(window.rem_size());
            let run = TextRun {
                len: self.language.len(),
                font,
                color: black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let label = window
                .text_system()
                .shape_line(self.language.clone().into(), font_size, &[run], None)
                .width();
            let actions = if self.controls.is_some() { 52. } else { 28. };
            let insets = rems(0.75).to_pixels(window.rem_size()) + px(2.);
            (width.measure(&self.copy_id, window, cx) + insets)
                .max(label + rems(actions / 16.).to_pixels(window.rem_size()) + insets)
                .ceil()
        });
        v_flex()
            .group("code-block")
            .relative()
            .min_w_0()
            .max_w_full()
            .self_start()
            .when_some(width, |this, width| this.w(width))
            .when(width.is_none(), |this| this.w_full())
            .surface_frame(cx)
            .bg(cx.theme().muted)
            .overflow_hidden()
            .gap_0p5()
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .px_1p5()
                    .py_0p5()
                    .gap_1()
                    .child(
                        div()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::ui::fade::faded(self.language)),
                    )
                    .child(div().flex_1())
                    .child(HoverCopy {
                        id: self.copy_id,
                        copy_source: self.copy_source,
                    })
                    .children(self.controls),
            )
            .child(div().w_full().min_w_0().px_1p5().pb_1p5().child(self.body))
            // Native child paint (Editor and source fades included) only
            // has a rectangular content mask. Keep the surface's corners
            // clear without changing layout or installing an input layer.
            .child(
                RoundedFrameCover::uniform(cx.theme().radius_lg)
                    .frame_border(px(1.))
                    .backdrop(cx.theme().background),
            )
    }
}

/// Keep the button in the layout and tab order while hidden. A focused child
/// reveals it too, so keyboard activation retains the native clipboard state.
#[derive(IntoElement)]
struct HoverCopy<F: Fn(&mut Window, &mut App) -> SharedString + 'static> {
    id: ElementId,
    copy_source: F,
}

impl<F: Fn(&mut Window, &mut App) -> SharedString + 'static> RenderOnce for HoverCopy<F> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = ElementId::NamedChild(std::sync::Arc::new(self.id.clone()), "hover".into());
        let focus = window
            .use_keyed_state(id.clone(), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        div()
            .id(id)
            .track_focus(&focus)
            .tab_stop(false)
            .flex_none()
            .opacity(if focus.contains_focused(window, cx) {
                1.
            } else {
                0.
            })
            .group_hover("code-block", |style| style.opacity(1.))
            .child(
                kit::Clipboard::new(self.id)
                    .value_fn(self.copy_source)
                    .hint("Копировать код", cx)
                    .xsmall(),
            )
    }
}

/// Lightweight source display retains native selection and horizontal scroll
/// without allocating the editor/highlighter used by ordinary Code widgets.
pub(super) fn source(id: impl Into<ElementId>, text: SharedString, cx: &App) -> AnyElement {
    source_body(id.into(), text, cx)
}

fn source_body(id: ElementId, text: SharedString, _: &App) -> AnyElement {
    Source { id, text }.into_any_element()
}

#[derive(IntoElement)]
struct Source {
    id: ElementId,
    text: SharedString,
}
impl RenderOnce for Source {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = window
            .use_keyed_state((self.id.clone(), "scroll"), cx, |_, _| ScrollHandle::new())
            .read(cx)
            .clone();
        div()
            .relative()
            .w_full()
            .min_w_0()
            .child(
                div()
                    .id(self.id)
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .track_scroll(&handle)
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_size(cx.theme().mono_font_size)
                    .line_height(relative(1.5))
                    .whitespace_nowrap()
                    .child(gpui_kit::base::SelectableText::new(
                        "source-text",
                        self.text,
                    )),
            )
            .child(super::code_geometry::source_fades(handle, cx))
    }
}

/// Missing providers retain an entity so element ids and native selection
/// survive source updates and stay scoped to this content node.
pub(super) struct SourceBlock {
    language: String,
    source: SharedString,
}

impl SourceBlock {
    pub(super) fn new(language: &str, source: &str) -> Self {
        Self {
            language: language.into(),
            source: source.to_owned().into(),
        }
    }

    pub(super) fn set_source(&mut self, source: &str, cx: &mut Context<Self>) {
        if self.source.as_ref() != source {
            self.source = source.to_owned().into();
            cx.notify();
        }
    }
}

impl Render for SourceBlock {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let copy_source = self.source.clone();
        shell(
            "copy-code-source",
            &self.language,
            move |_, _| copy_source.clone(),
            None,
            source("code-source-scroll", self.source.clone(), cx),
            cx,
        )
        .fit(Width::Source(self.source.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::preview_rows;

    #[test]
    fn preview_rows_omit_only_the_terminal_cursor_row() {
        for (source, rows) in [
            ("", 1),
            ("\n", 1),
            ("\n\n", 2),
            ("a", 1),
            ("a\nb", 2),
            ("a\nb\n", 2),
            ("a\nb\n\n", 3),
            ("a\r\nb\r\n", 2),
            ("a\rb", 1),
            ("λ\n🚀\n", 2),
        ] {
            assert_eq!(preview_rows(source), rows, "{source:?}");
        }
    }
}
