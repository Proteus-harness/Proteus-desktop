// Адаптировано из Zed: crates/markdown/src/markdown.rs (code surfaces/copy),
// crates/markdown/src/mermaid.rs (source/preview controls).
//! One native fence shell for code, contributed previews and source fallback.
//! Only the visible body is mounted; the shell never creates an editor or a
//! renderer. Full-document content deliberately bypasses it.

use super::code_geometry::Width;
use crate::ui::kit::{self, Frame as _, Hint as _};
use gpui_kit::base::RoundedFrameCover;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Plain prose fences have no gutter; programming languages keep the
/// existing unwrapped editor presentation. This policy also applies when a
/// retained Code node changes its language without changing its id.
pub(super) fn plain_language(language: &str) -> bool {
    [
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

pub(super) fn line_numbers(language: &str, wrap: bool) -> bool {
    !wrap && !plain_language(language)
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
        disclosure: None,
        minimum_disclosure_label: None,
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
    disclosure: Option<AnyElement>,
    minimum_disclosure_label: Option<SharedString>,
}

impl<F: Fn(&mut Window, &mut App) -> SharedString + 'static> Shell<F> {
    pub(super) fn fit(mut self, width: Width) -> Self {
        self.width = Some(width);
        self
    }

    /// A disclosure shares the language/copy row. Reserve its longest label
    /// so short source rows and opening/closing never clip or shift actions.
    pub(super) fn disclosure(
        mut self,
        disclosure: AnyElement,
        label: impl Into<SharedString>,
    ) -> Self {
        self.disclosure = Some(disclosure);
        self.minimum_disclosure_label = Some(label.into());
        self
    }
}

impl<F: Fn(&mut Window, &mut App) -> SharedString + 'static> RenderOnce for Shell<F> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Keep the language and action slots readable even for an empty fence.
        // These are the same text_xs and xsmall icon sizes as the header below.
        let font = window.text_style().font();
        let font_size = rems(0.75).to_pixels(window.rem_size());
        let measure_label = |label: &str| {
            let run = TextRun {
                len: label.len(),
                font: font.clone(),
                color: black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_line(label.to_owned().into(), font_size, &[run], None)
                .width()
        };
        let disclosure_width = self
            .minimum_disclosure_label
            .as_ref()
            .map(|label| measure_label(label) + rems(0.75 + 0.25).to_pixels(window.rem_size()));
        let label = if self.width.is_some() {
            measure_label(&self.language)
        } else {
            px(0.)
        };
        let width = self.width.as_ref().map(|width| {
            let disclosure =
                disclosure_width.map(|width| width + rems(0.25).to_pixels(window.rem_size()));
            let actions = if self.controls.is_some() { 52. } else { 28. };
            fit_width(
                width.measure(&self.copy_id, window, cx),
                label
                    + rems(actions / 16.).to_pixels(window.rem_size())
                    + disclosure.unwrap_or(px(0.)),
                window,
            )
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
                    .children(self.disclosure.map(|disclosure| {
                        div()
                            .flex_none()
                            .when_some(disclosure_width, |slot, width| slot.w(width))
                            .child(disclosure)
                    }))
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

/// Reserve the same individually snapped sides that the shell will lay out.
/// The final width rounds outward on the device grid, so fractional glyph
/// advances cannot become a tiny scroll range after the frame is fitted.
pub(super) fn fit_width(body: Pixels, header: Pixels, window: &Window) -> Pixels {
    let padding = window.pixel_snap(rems(0.375).to_pixels(window.rem_size()));
    let border = window
        .pixel_snap(px(1.))
        .max(px(1. / window.scale_factor()));
    let width = body.max(header) + (padding + border) * 2.;
    (width * window.scale_factor()).ceil() / window.scale_factor()
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

#[derive(Default)]
struct SourceScroll {
    handle: ScrollHandle,
    horizontal_overflow: bool,
}

impl RenderOnce for Source {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state((self.id.clone(), "scroll"), cx, |_, _| {
            SourceScroll::default()
        });
        source_view(self.id, self.text, state, window, cx)
    }
}

fn source_view(
    id: ElementId,
    text: SharedString,
    state: Entity<SourceScroll>,
    window: &mut Window,
    cx: &App,
) -> Div {
    let handle = state.read(cx).handle.clone();
    let overflowing = state.read(cx).horizontal_overflow;
    let track = if overflowing {
        super::code_geometry::horizontal_track_height(cx)
    } else {
        px(0.)
    };
    let observed = handle.clone();
    let owner = window.current_view();
    div()
        .relative()
        .w_full()
        .min_w_0()
        .pb(track)
        .child(
            div()
                .id(id)
                .flex()
                .w_full()
                .min_w_0()
                .overflow_x_scroll()
                // A source fence owns X only. GPUI otherwise redirects a
                // plain vertical wheel onto X and keeps it from the chat.
                .restrict_scroll_to_axis()
                .track_scroll(&handle)
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .line_height(relative(1.5))
                .whitespace_nowrap()
                .child(
                    div()
                        .flex_none()
                        .child(gpui_kit::base::SelectableText::new("source-text", text)),
                ),
        )
        .child(super::code_geometry::source_fades(
            handle.clone(),
            track,
            cx,
        ))
        .when(overflowing, |view| {
            view.child(
                div().absolute().inset_0().child(
                    Scrollbar::horizontal(&handle)
                        .viewport_from_layout()
                        .styles(|styles| styles.track(|style| style.width(track))),
                ),
            )
        })
        .child(
            canvas(
                move |_, _, cx| {
                    let horizontal = observed.max_offset().x > px(0.);
                    if state.read(cx).horizontal_overflow != horizontal {
                        state.update(cx, |state, _| state.horizontal_overflow = horizontal);
                        cx.notify(owner);
                    }
                },
                |_, _, _, _| {},
            )
            // An overflow observer must not add content to its own area.
            .absolute()
            .top_0()
            .left_0()
            .size(px(0.)),
        )
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
#[path = "code_block/source_tests.rs"]
mod source_tests;

#[cfg(test)]
mod tests {
    use super::{line_numbers, plain_language, preview_rows};

    #[test]
    fn prose_fence_languages_share_the_plain_body_policy() {
        for language in [
            "",
            "md",
            "MARKDOWN",
            " text ",
            "PlainText",
            "TXT",
            "text/plain",
            "Text/Markdown",
        ] {
            assert!(plain_language(language), "{language:?}");
            assert!(!line_numbers(language, false), "{language:?}");
        }
        for language in ["rust", "python", "json", "mermaid", "latex", "textile"] {
            assert!(!plain_language(language), "{language:?}");
            assert!(line_numbers(language, false), "{language:?}");
            assert!(!line_numbers(language, true), "{language:?}");
        }
    }

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
