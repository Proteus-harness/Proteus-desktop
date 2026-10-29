//! Retained native text and nested content. Producers choose formatting and
//! language; this adapter only mounts public widgets and keeps selection.

use super::{content::ContentView, view::DeclarativeView};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::*;

pub(super) struct RichText {
    state: Entity<TextViewState>,
    html: String,
    _changed: Subscription,
}

impl RichText {
    fn new(html: &str, cx: &mut Context<DeclarativeView>) -> Self {
        let state = cx.new(|cx| TextViewState::html(html, cx).scrollable(false));
        let changed = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            html: html.into(),
            _changed: changed,
        }
    }

    fn set_html(&mut self, html: &str, cx: &mut Context<DeclarativeView>) {
        if self.html != html {
            self.html = html.into();
            self.state.update(cx, |state, cx| state.set_text(html, cx));
        }
    }

    pub(super) fn element(&self) -> AnyElement {
        TextView::new(&self.state)
            .selectable(true)
            .into_any_element()
    }
}

pub(super) struct Content {
    pub(super) view: Option<Entity<ContentView>>,
    language: String,
    renderer: Option<&'static str>,
    fallback: Option<SharedString>,
    _changed: Option<Subscription>,
    _resize: Option<Subscription>,
}

impl Content {
    pub(super) fn element(&self) -> AnyElement {
        match &self.view {
            Some(view) => view.clone().into_any_element(),
            None => self
                .fallback
                .as_ref()
                .map(|text| {
                    gpui_kit::base::SelectableText::new("content-fallback", text.clone())
                        .into_any_element()
                })
                .unwrap_or_else(|| div().into_any_element()),
        }
    }
}

impl DeclarativeView {
    pub(super) fn prepare_rich_text(&mut self, id: &str, html: &str, cx: &mut Context<Self>) {
        match self.fields.rich_text.get_mut(id) {
            Some(text) => text.set_html(html, cx),
            None => {
                self.fields
                    .rich_text
                    .insert(id.into(), RichText::new(html, cx));
            }
        }
    }

    pub(super) fn prepare_content(
        &mut self,
        id: &str,
        language: &str,
        source: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let renderer = super::content::language_renderer(language, cx);
        if let Some(content) = self.fields.contents.get_mut(id)
            && content.language == language
            && content.renderer == renderer
        {
            if let Some(view) = &content.view {
                view.update(cx, |view, cx| view.set_source(language, source, window, cx));
            } else {
                content.fallback = Some(source.to_owned().into());
            }
            return;
        }
        let view = self
            .host(cx)
            .and_then(|host| ContentView::child(host, id.into(), language, source, window, cx));
        let changed = view
            .as_ref()
            .map(|view| cx.observe(view, |_, _, cx| cx.notify()));
        let resize = view.as_ref().map(|view| {
            cx.subscribe(view, |_, _, _: &super::content::WillResize, cx| {
                cx.emit(super::view::WillResize);
            })
        });
        let fallback = view
            .is_none()
            .then(|| SharedString::from(source.to_owned()));
        let content = Content {
            language: language.into(),
            renderer,
            view,
            fallback,
            _changed: changed,
            _resize: resize,
        };
        self.fields.contents.insert(id.into(), content);
    }
}

/// The native editor consumes no vertical wheel input when every physical row
/// fits. Keep the code unwrapped and disable its editor-only tail/overscroll.
pub(super) fn embedded_code(
    id: &str,
    text: &str,
    language: &str,
    state: Option<&Entity<gpui_kit::component::input::EditorState>>,
    cx: &mut Context<DeclarativeView>,
) -> AnyElement {
    use crate::ui::kit::{Frame as _, Hint as _};
    use gpui_kit::component::{ActiveTheme as _, Sizable as _};
    let rows = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
    // Editor uses the same mono font and 1.5 leading; an extra physical row
    // covers native input insets at all interface scales.
    let height = cx.theme().mono_font_size * (1.5 * (rows + 1) as f32);
    v_flex()
        .w_full()
        .min_w_0()
        .surface_frame(cx)
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1()
                .gap_1()
                .child(div().text_xs().child(language.to_owned()))
                .child(div().flex_1())
                .child(
                    crate::ui::kit::Clipboard::new(SharedString::from(format!("code-copy-{id}")))
                        .value(text.to_owned())
                        .hint("Копировать код", cx)
                        .xsmall(),
                ),
        )
        .children(state.map(|state| {
            gpui_kit::component::input::Editor::new(state)
                .readonly(true)
                .appearance(false)
                .bordered(false)
                .h(height)
        }))
        .into_any_element()
}
