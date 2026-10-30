//! Retained atomic content slots in selectable HTML. Routing stays with the
//! package's language declarations; native text owns wrapping and selection.

use super::{
    content::{self, ContentView},
    view::DeclarativeView,
};
use crate::declarative::widget::InlineContent;
use crate::ui::extensions::Host;
use gpui_kit::base::text::{MarkdownExtensions, MarkdownNode};
use gpui_kit::*;
use std::{collections::HashMap, sync::Arc};

struct Slot {
    language: String,
    renderer: Option<&'static str>,
    view: Option<Entity<ContentView>>,
    _changed: Option<Subscription>,
}

#[derive(Default)]
pub(super) struct Slots {
    slots: HashMap<String, Slot>,
    pub(super) extensions: MarkdownExtensions,
}

impl Slots {
    pub(super) fn sync(
        &mut self,
        id: &str,
        specifications: &[InlineContent],
        host: Option<Host>,
        text: &Entity<gpui_kit::component::text::TextViewState>,
        window: &mut Window,
        cx: &mut Context<DeclarativeView>,
    ) {
        if specifications.is_empty() && self.slots.is_empty() {
            return;
        }
        let before = self.slots.len();
        self.slots
            .retain(|id, _| specifications.iter().any(|slot| slot.id == *id));
        let mut changed = before != self.slots.len();
        let languages = content::languages(cx);
        for specification in specifications {
            let renderer = languages
                .iter()
                .find(|(language, _)| language == &specification.language)
                .map(|(_, renderer)| *renderer);
            if let Some(slot) = self.slots.get(&specification.id)
                && slot.language == specification.language
                && slot.renderer == renderer
            {
                if let Some(view) = &slot.view {
                    view.update(cx, |view, cx| {
                        view.set_source(&specification.language, &specification.source, window, cx)
                    });
                }
                continue;
            }
            let view = host.clone().and_then(|host| {
                ContentView::inline(
                    host,
                    format!("{id}:{}", specification.id),
                    &specification.language,
                    &specification.source,
                    window,
                    cx,
                )
            });
            let text = text.clone();
            let changed_subscription = view.as_ref().map(|view| {
                cx.observe(view, move |_, _, cx| {
                    // Async SVG completion changes line height and the parent chat row.
                    text.update(cx, |_, cx| cx.notify());
                    cx.notify();
                })
            });
            self.slots.insert(
                specification.id.clone(),
                Slot {
                    language: specification.language.clone(),
                    renderer,
                    view,
                    _changed: changed_subscription,
                },
            );
            changed = true;
        }
        if changed {
            let views: HashMap<_, _> = self
                .slots
                .iter()
                .filter_map(|(id, slot)| slot.view.as_ref().map(|view| (id.clone(), view.clone())))
                .collect();
            let views = Arc::new(views);
            self.extensions = MarkdownExtensions::default()
                .html_inline("data-content", |id, fallback| {
                    Some(
                        MarkdownNode::new("content", id.to_owned())
                            .text(fallback.to_owned())
                            .markdown(fallback.to_owned()),
                    )
                })
                .inline_renderer("content", move |node, context, window, cx| {
                    let view = views.get(node.data::<String>()?)?;
                    view.update(cx, |view, cx| view.inline_element(context, window, cx))
                });
        }
    }

    pub(super) fn rehost(
        &self,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        window: &mut Window,
        cx: &mut Context<DeclarativeView>,
    ) {
        for view in self.slots.values().filter_map(|slot| slot.view.as_ref()) {
            view.update(cx, |view, cx| view.rehost(workspace.clone(), window, cx));
        }
    }
}
