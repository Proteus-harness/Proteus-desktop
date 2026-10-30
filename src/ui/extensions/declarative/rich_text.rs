//! Retained native text and nested content. Producers choose formatting and
//! language; this adapter only mounts public widgets and keeps selection.

use super::{code_block, content::ContentView, view::DeclarativeView};
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::*;

pub(super) struct RichText {
    state: Entity<TextViewState>,
    html: String,
    slots: super::inline_content::Slots,
    _changed: Subscription,
}

impl RichText {
    fn new(html: &str, cx: &mut Context<DeclarativeView>) -> Self {
        let state = cx.new(|cx| TextViewState::html(html, cx).scrollable(false));
        let changed = cx.observe(&state, |_, _, cx| cx.notify());
        Self {
            state,
            html: html.into(),
            slots: super::inline_content::Slots::default(),
            _changed: changed,
        }
    }

    fn set_html(&mut self, html: &str, cx: &mut Context<DeclarativeView>) {
        if self.html != html {
            self.html = html.into();
            self.state.update(cx, |state, cx| state.set_text(html, cx));
        }
    }

    pub(super) fn rehost(
        &self,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        window: &mut Window,
        cx: &mut Context<DeclarativeView>,
    ) {
        self.slots.rehost(workspace, window, cx);
    }

    pub(super) fn element(&self) -> AnyElement {
        TextView::new(&self.state)
            .selectable(true)
            .markdown_extensions(self.slots.extensions.clone())
            .into_any_element()
    }
}

pub(super) struct Content {
    pub(super) view: Option<Entity<ContentView>>,
    language: String,
    renderer: Option<&'static str>,
    fallback: Option<Entity<code_block::SourceBlock>>,
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
                .map(|view| view.clone().into_any_element())
                .unwrap_or_else(|| div().into_any_element()),
        }
    }
}

impl DeclarativeView {
    pub(super) fn prepare_rich_text(
        &mut self,
        id: &str,
        html: &str,
        inlines: &[crate::declarative::widget::InlineContent],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = self.host(cx);
        let mut text = self
            .fields
            .rich_text
            .remove(id)
            .unwrap_or_else(|| RichText::new(html, cx));
        text.set_html(html, cx);
        text.slots.sync(id, inlines, host, &text.state, window, cx);
        self.fields.rich_text.insert(id.into(), text);
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
            } else if let Some(fallback) = &content.fallback {
                fallback.update(cx, |view, cx| view.set_source(source, cx));
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
            .then(|| cx.new(|_| code_block::SourceBlock::new(language, source)));
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
    use gpui_kit::component::ActiveTheme as _;
    let rows = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
    // Editor uses the same mono font and 1.5 leading; an extra physical row
    // covers native input insets at all interface scales.
    let height = cx.theme().mono_font_size * (1.5 * (rows + 1) as f32);
    let body = state
        .map(|state| {
            code_block::readonly_editor(state, cx)
                .h(height)
                .into_any_element()
        })
        .unwrap_or_else(|| {
            code_block::source(
                SharedString::from(format!("code-source-{id}")),
                text.to_owned().into(),
                cx,
            )
        });
    code_block::shell(
        SharedString::from(format!("code-copy-{id}")),
        language,
        text,
        None,
        body,
        cx,
    )
    .into_any_element()
}
