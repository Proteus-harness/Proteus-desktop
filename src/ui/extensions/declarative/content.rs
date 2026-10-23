//! Generic retained chat content surface. Language registration belongs to
//! packages; the native shell only owns source, copy, fallback and lifetime.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::view::DeclarativeView;
use crate::declarative::protocol::{Content, SurfaceKind};
use crate::ui::extensions::Host;
use crate::ui::kit;
use crate::ui::kit::Hint as _;

/// Enabled contributors in the user's extension order; the first claimant
/// wins a duplicate language registration deterministically.
pub(in crate::ui) fn languages(cx: &App) -> Vec<(String, &'static str)> {
    crate::ui::extensions::ordered(cx)
        .into_iter()
        .filter(|manifest| crate::ui::extensions::is_enabled(manifest.id, cx))
        .flat_map(|manifest| {
            super::installed(manifest.id)
                .into_iter()
                .filter_map(|extension| extension.manifest.surfaces.content.as_ref())
                .flat_map(move |content| {
                    content
                        .languages
                        .iter()
                        .cloned()
                        .map(move |language| (language, manifest.id))
                })
        })
        .collect()
}

pub(in crate::ui) struct WillResize;

impl EventEmitter<WillResize> for ContentView {}

pub(in crate::ui) struct ContentView {
    rendered: Entity<DeclarativeView>,
    source: Entity<TextViewState>,
    content: Content,
    extension: &'static str,
    show_source: bool,
    _rendered: Subscription,
    _source: Subscription,
}

impl ContentView {
    pub(in crate::ui) fn new(
        host: Host,
        id: String,
        content: Content,
        fence: Arc<str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let extension = host.extension();
        let runtime = super::runtime(&host, window, cx).expect("registered content extension");
        let rendered = cx.new(|cx| {
            DeclarativeView::with_content(
                runtime,
                SurfaceKind::Content,
                Some(id),
                Some(content.clone()),
                cx,
            )
        });
        let source = cx.new(|cx| TextViewState::markdown(&fence, cx));
        Self {
            _rendered: cx.observe(&rendered, |_, _, cx| {
                cx.emit(WillResize);
                cx.notify();
            }),
            _source: cx.observe(&source, |_, _, cx| cx.notify()),
            rendered,
            source,
            content,
            extension,
            show_source: false,
        }
    }
}

impl Render for ContentView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = crate::ui::extensions::is_enabled(self.extension, cx);
        let view = self.rendered.read(cx);
        let ready = enabled && view.ready(cx);
        let error = view.error(cx);
        let raw = self.show_source || !ready;
        let header = h_flex()
            .w_full()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(crate::ui::fade::faded(self.content.language.clone())),
            )
            .child(div().flex_1())
            .child(
                kit::Clipboard::new("copy-content-source")
                    .value(self.content.source.clone())
                    .hint("Копировать исходник", cx)
                    .xsmall(),
            )
            .child(
                kit::button("content-source")
                    .icon(IconName::Code)
                    .xsmall()
                    .ghost()
                    .selected(self.show_source)
                    .disabled(!ready)
                    .hint(
                        if self.show_source {
                            "Показать визуализацию"
                        } else {
                            "Показать исходник"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.emit(WillResize);
                        this.show_source = !this.show_source;
                        cx.notify();
                    })),
            );
        v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
            .child(header)
            .when_some(error, |view, error| {
                view.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(crate::ui::fade::faded(format!(
                            "Визуализация недоступна: {error}"
                        ))),
                )
            })
            .child(if raw {
                TextView::new(&self.source)
                    .selectable(true)
                    .into_any_element()
            } else {
                self.rendered.clone().into_any_element()
            })
    }
}
