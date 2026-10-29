// Адаптировано из Zed: crates/markdown/src/markdown.rs (code surfaces/copy),
// crates/markdown/src/mermaid.rs (source/preview controls).
//! One native fence shell for code, contributed previews and source fallback.
//! Only the visible body is mounted; the shell never creates an editor or a
//! renderer. Full-document content deliberately bypasses it.

use crate::ui::kit::{self, Frame as _, Hint as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

/// Shared code presentation for embedded and standalone readonly widgets.
/// Reuses the retained editor while keeping its body and opaque gutter on the
/// same surface as the fence shell.
pub(super) fn readonly_editor(
    state: &Entity<gpui_kit::component::input::EditorState>,
    cx: &App,
) -> gpui_kit::component::input::Editor {
    gpui_kit::component::input::Editor::new(state)
        .readonly(true)
        .appearance(false)
        .bordered(false)
        .bg(cx.theme().muted)
}

pub(super) fn shell(
    copy_id: impl Into<ElementId>,
    language: &str,
    source: &str,
    controls: Option<AnyElement>,
    body: AnyElement,
    cx: &App,
) -> Div {
    v_flex()
        .w_full()
        .min_w_0()
        .surface_frame(cx)
        .bg(cx.theme().muted)
        .overflow_hidden()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .min_w_0()
                .items_center()
                .px_2()
                .py_1()
                .gap_1()
                .child(
                    div()
                        .min_w_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::ui::fade::faded(language.to_owned())),
                )
                .child(div().flex_1())
                .child(
                    kit::Clipboard::new(copy_id)
                        .value(source.to_owned())
                        .hint("Копировать код", cx)
                        .xsmall(),
                )
                .children(controls),
        )
        .child(div().w_full().min_w_0().px_2().pb_2().child(body))
}

/// Lightweight source display retains native selection and horizontal scroll
/// without allocating the editor/highlighter used by ordinary Code widgets.
pub(super) fn source(id: impl Into<ElementId>, text: SharedString, cx: &App) -> AnyElement {
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .overflow_x_scroll()
        .font_family(cx.theme().mono_font_family.clone())
        .text_size(cx.theme().mono_font_size)
        .line_height(relative(1.5))
        .whitespace_nowrap()
        .child(gpui_kit::base::SelectableText::new("source-text", text))
        .into_any_element()
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
        shell(
            "copy-code-source",
            &self.language,
            &self.source,
            None,
            source("code-source-scroll", self.source.clone(), cx),
            cx,
        )
    }
}
