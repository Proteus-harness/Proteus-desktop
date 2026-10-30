// Адаптировано из Zed: crates/markdown/src/markdown.rs (code surfaces/copy),
// crates/markdown/src/mermaid.rs (source/preview controls).
//! One native fence shell for code, contributed previews and source fallback.
//! Only the visible body is mounted; the shell never creates an editor or a
//! renderer. Full-document content deliberately bypasses it.

use crate::ui::kit::{self, Frame as _, Hint as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
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

pub(super) fn shell(
    copy_id: impl Into<ElementId>,
    language: &str,
    source: &str,
    controls: Option<AnyElement>,
    body: AnyElement,
    cx: &App,
) -> Div {
    v_flex()
        .group("code-block")
        .w_full()
        .min_w_0()
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
                        .child(crate::ui::fade::faded(language.to_owned())),
                )
                .child(div().flex_1())
                .child(HoverCopy {
                    id: copy_id.into(),
                    source: source.to_owned().into(),
                })
                .children(controls),
        )
        .child(div().w_full().min_w_0().px_1p5().pb_1p5().child(body))
}

/// Keep the button in the layout and tab order while hidden. A focused child
/// reveals it too, so keyboard activation retains the native clipboard state.
#[derive(IntoElement)]
struct HoverCopy {
    id: ElementId,
    source: SharedString,
}

impl RenderOnce for HoverCopy {
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
                    .value(self.source)
                    .hint("Копировать код", cx)
                    .xsmall(),
            )
    }
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
