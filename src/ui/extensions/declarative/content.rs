//! Generic retained chat content surface. Language registration belongs to
//! packages; the native shell only owns source, copy, fallback and lifetime.

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{code_block, view::DeclarativeView};
use crate::declarative::protocol::{Content, SurfaceKind};
use crate::ui::extensions::Host;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
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

pub(in crate::ui) fn language_renderer(language: &str, cx: &App) -> Option<&'static str> {
    languages(cx)
        .into_iter()
        .find(|(name, _)| name == language)
        .map(|(_, id)| id)
}

/// Full-body contributors use an explicit media type rather than a package id.
pub(in crate::ui) fn format(media_type: &str, cx: &App) -> Option<&'static str> {
    crate::ui::extensions::ordered(cx)
        .into_iter()
        .filter(|manifest| crate::ui::extensions::is_enabled(manifest.id, cx))
        .find_map(|manifest| {
            super::installed(manifest.id)?
                .manifest
                .surfaces
                .content
                .as_ref()?
                .formats
                .iter()
                .any(|format| format == media_type)
                .then_some(manifest.id)
        })
}

fn populate_context(value: &mut serde_json::Value, cx: &App) {
    let mut context = value.as_object().cloned().unwrap_or_default();
    let languages = languages(cx);
    let mut contributors = serde_json::Map::new();
    for (language, id) in &languages {
        contributors
            .entry(language.clone())
            .or_insert_with(|| serde_json::json!(id));
    }
    context.insert(
        "languages".into(),
        serde_json::json!(
            languages
                .into_iter()
                .map(|(language, _)| language)
                .collect::<Vec<_>>()
        ),
    );
    context.insert("contributors".into(), contributors.into());
    *value = context.into();
}

/// Keep one immutable latest payload shared by the shell and runtime. Compare
/// borrowed text before allocating; the wire copy belongs after queue admission.
fn updated_content(
    current: &Content,
    language: &str,
    source: &str,
    context: serde_json::Value,
) -> Option<Arc<Content>> {
    if current.language == language && current.source == source && current.context == context {
        return None;
    }
    Some(Arc::new(Content {
        language: language.into(),
        source: source.into(),
        context,
        source_revision: Some(current.source_revision.unwrap_or_default().wrapping_add(1)),
    }))
}

pub(in crate::ui) struct WillResize;

impl EventEmitter<WillResize> for ContentView {}

pub(in crate::ui) struct ContentView {
    rendered: Entity<DeclarativeView>,
    content: Arc<Content>,
    extension: &'static str,
    show_source: bool,
    body: bool,
    _rendered: Subscription,
    _resize: Subscription,
}

impl ContentView {
    pub(in crate::ui) fn new(
        host: Host,
        id: String,
        mut content: Content,
        _fence: Arc<str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        populate_context(&mut content.context, cx);
        let content = Arc::new(content);
        let extension = host.extension();
        let runtime = super::runtime(&host, window, cx).expect("registered content extension");
        let rendered = cx.new(|cx| {
            DeclarativeView::with_content(
                runtime,
                host,
                SurfaceKind::Content,
                Some(id),
                Some(content.clone()),
                window,
                cx,
            )
        });
        Self {
            _rendered: cx.observe(&rendered, |_, _, cx| cx.notify()),
            _resize: cx.subscribe(&rendered, |_, _, _: &super::view::WillResize, cx| {
                cx.emit(WillResize);
            }),
            rendered,
            content,
            extension,
            show_source: false,
            body: false,
        }
    }

    /// An ordinary reply already owns its copy actions in the chat row.
    pub(in crate::ui) fn body(
        host: Host,
        id: String,
        content: Content,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new(host, id, content, Arc::from(""), window, cx);
        view.body = true;
        view
    }

    /// Nested content routing is generic; a renderer never names a package.
    pub(in crate::ui) fn child(
        host: Host,
        id: String,
        language: &str,
        source: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<Self>> {
        let extension = language_renderer(language, cx)?;
        let host = host.with_extension(extension);
        let content = Content {
            language: language.into(),
            source: source.into(),
            context: serde_json::Value::Null,
            source_revision: Some(0),
        };
        Some(cx.new(|cx| Self::new(host, id, content, Arc::from(""), window, cx)))
    }

    pub(in crate::ui) fn set_source(
        &mut self,
        language: &str,
        source: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut context = self.content.context.clone();
        populate_context(&mut context, cx);
        let Some(content) = updated_content(&self.content, language, source, context) else {
            return;
        };
        self.content = content.clone();
        self.rendered
            .update(cx, |view, cx| view.set_content(content, window, cx));
        cx.notify();
    }

    pub(in crate::ui) fn rehost(
        &mut self,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rendered
            .update(cx, |view, cx| view.rehost(workspace, window, cx));
        cx.notify();
    }
}

#[cfg(test)]
mod tests;

impl Render for ContentView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = crate::ui::extensions::is_enabled(self.extension, cx);
        let view = self.rendered.read(cx);
        let ready = enabled && view.ready(cx);
        let error = view.error(cx);
        let raw = self.show_source || !ready;
        let body = v_flex()
            .w_full()
            .min_w_0()
            .gap_1()
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
                let source = SharedString::from(self.content.source.clone());
                if self.body {
                    gpui_kit::base::SelectableText::new("content-source-text", source)
                        .into_any_element()
                } else {
                    code_block::source("content-source-scroll", source, cx)
                }
            } else {
                self.rendered.clone().into_any_element()
            });
        if self.body {
            return body.into_any_element();
        }
        let toggle = kit::button("content-source")
            .icon(IconName::Code)
            .xsmall()
            .quiet(cx)
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
            }));
        code_block::shell(
            "copy-content-source",
            &self.content.language,
            &self.content.source,
            Some(toggle.into_any_element()),
            body.into_any_element(),
            cx,
        )
        .into_any_element()
    }
}

/// Retained full-body content outside the chat, e.g. an installed package's
/// README. Creation needs no window; the contributor starts on first render.
pub(in crate::ui) struct FormattedText {
    workspace: WeakEntity<crate::ui::workspace::Workspace>,
    format: String,
    source: SharedString,
    contributor: Option<&'static str>,
    view: Option<Entity<ContentView>>,
    changes: Option<Subscription>,
}

impl FormattedText {
    pub(in crate::ui) fn new(
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        format: &str,
        source: String,
    ) -> Self {
        Self {
            workspace,
            format: format.into(),
            source: source.into(),
            contributor: None,
            view: None,
            changes: None,
        }
    }
}

impl Render for FormattedText {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use crate::ui::kit::SelectionCopyMenu as _;
        let contributor = format(&self.format, cx);
        if self.contributor != contributor {
            self.view = contributor.map(|id| {
                let content = Content {
                    language: self.format.clone(),
                    source: self.source.to_string(),
                    context: serde_json::Value::Null,
                    source_revision: Some(0),
                };
                cx.new(|cx| {
                    ContentView::body(
                        Host::new(self.workspace.clone(), id),
                        "text".into(),
                        content,
                        window,
                        cx,
                    )
                })
            });
            self.changes = self
                .view
                .as_ref()
                .map(|view| cx.observe(view, |_, _, cx| cx.notify()));
            self.contributor = contributor;
        }
        div()
            .w_full()
            .min_w_0()
            .child(match &self.view {
                Some(view) => view.clone().into_any_element(),
                None => gpui_kit::base::SelectableText::new("plain-source", self.source.clone())
                    .into_any_element(),
            })
            .selection_copy_menu()
            .w_full()
    }
}
