//! Retained generic reply surfaces. Format policy and parsing belong to the
//! contributing process; the chat only supplies source and native lifetime.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::*;

use super::Chat;
use crate::declarative::protocol::Content;
use crate::ui::extensions::{Host, content};
use crate::ui::workspace::Workspace;

#[derive(Default)]
pub(super) struct Replies(HashMap<u64, Reply>);

#[derive(Clone)]
pub(super) enum Part {
    Plain(Arc<str>),
    Content(Entity<content::ContentView>),
}

struct Reply {
    text: Arc<str>,
    languages: Vec<(String, &'static str)>,
    renderer: Option<&'static str>,
    part: Part,
    _changes: Option<Subscription>,
    _resize: Option<Subscription>,
}

impl Replies {
    pub(super) fn parts(
        &mut self,
        key: u64,
        text: &Arc<str>,
        workspace: WeakEntity<Workspace>,
        session: PathBuf,
        window: &mut Window,
        cx: &mut Context<Chat>,
    ) -> Vec<Part> {
        let renderer = content::format("text/markdown", cx);
        let languages = content::languages(cx);
        if let Some(reply) = self.0.get_mut(&key)
            && reply.renderer == renderer
        {
            if !Arc::ptr_eq(&reply.text, text) || reply.languages != languages {
                match &mut reply.part {
                    Part::Content(view) => view.update(cx, |view, cx| {
                        view.set_source("text/markdown", text, window, cx);
                    }),
                    Part::Plain(plain) => *plain = text.clone(),
                }
                reply.text = text.clone();
                reply.languages = languages;
            }
            return vec![reply.part.clone()];
        }
        let (part, changes, resize) = if let Some(extension) = renderer {
            let payload = Content {
                language: "text/markdown".into(),
                source: text.to_string(),
                context: serde_json::Value::Null,
                source_revision: Some(0),
            };
            let host = Host::new(workspace, extension).for_chat(cx.weak_entity(), session);
            let view =
                cx.new(|cx| content::ContentView::body(host, key.to_string(), payload, window, cx));
            let changes = cx.observe(&view, move |chat, _, cx| chat.remeasure_row(key, cx));
            let resize = cx.subscribe(&view, move |chat, _, _: &content::WillResize, cx| {
                if let Some(row) = chat.row_of(key) {
                    chat.scroller
                        .update(cx, |scroller, cx| scroller.toggled(row..row + 1, cx));
                }
            });
            (Part::Content(view), Some(changes), Some(resize))
        } else {
            (Part::Plain(text.clone()), None, None)
        };
        self.0.insert(
            key,
            Reply {
                text: text.clone(),
                languages,
                renderer,
                part: part.clone(),
                _changes: changes,
                _resize: resize,
            },
        );
        vec![part]
    }

    pub(super) fn rehost(
        &mut self,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Chat>,
    ) {
        for reply in self.0.values_mut() {
            if let Part::Content(view) = &reply.part {
                view.update(cx, |view, cx| view.rehost(workspace.clone(), window, cx));
            }
        }
    }

    pub(super) fn retain(&mut self, keys: &HashSet<u64>) {
        // Releasing historical rows closes their surfaces and child workers.
        self.0.retain(|key, _| keys.contains(key));
    }
}
