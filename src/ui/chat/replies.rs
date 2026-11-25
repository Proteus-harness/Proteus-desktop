//! Retained generic reply surfaces. Format policy and parsing belong to the
//! contributing process; the chat only supplies source and native lifetime.

use crate::tabs::ChatKey;
use std::collections::{HashMap, HashSet};
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
    languages: Vec<(&'static str, &'static str)>,
    renderer: &'static str,
    media_type: &'static str,
    view: Entity<content::ContentView>,
    _changes: Subscription,
    _resize: Subscription,
}

impl Replies {
    pub(super) fn part(
        &mut self,
        key: u64,
        media_type: &'static str,
        text: &Arc<str>,
        workspace: WeakEntity<Workspace>,
        session: ChatKey,
        readonly: bool,
        window: &mut Window,
        cx: &mut Context<Chat>,
    ) -> Part {
        let Some(renderer) = content::format(media_type, cx) else {
            // Plain text is already shared by the transcript entry. Only
            // renderer views carry state worth retaining between frames.
            self.0.remove(&key);
            return Part::Plain(text.clone());
        };
        let languages = content::languages(cx);
        if let Some(reply) = self.0.get_mut(&key)
            && reply.renderer == renderer
            && reply.media_type == media_type
        {
            if !Arc::ptr_eq(&reply.text, text) || reply.languages != languages {
                reply.view.update(cx, |view, cx| {
                    view.set_source(media_type, text, window, cx);
                });
                reply.text = text.clone();
                reply.languages = languages;
            }
            return Part::Content(reply.view.clone());
        }
        let payload = Content {
            language: media_type.into(),
            source: text.into(),
            context: serde_json::Value::Null,
            source_revision: Some(0),
        };
        let host = Host::new(workspace, renderer).for_chat(cx.weak_entity(), session);
        let host = if readonly { host.content_only() } else { host };
        let view =
            cx.new(|cx| content::ContentView::body(host, key.to_string(), payload, window, cx));
        let changes = cx.observe(&view, move |chat, _, cx| chat.remeasure_row(key, cx));
        let resize = cx.subscribe(&view, move |chat, _, _: &content::WillResize, cx| {
            if let Some(row) = chat.row_of(key) {
                chat.scroller
                    .update(cx, |scroller, cx| scroller.toggled(row..row + 1, true, cx));
            }
        });
        self.0.insert(
            key,
            Reply {
                text: text.clone(),
                languages,
                renderer,
                media_type,
                view: view.clone(),
                _changes: changes,
                _resize: resize,
            },
        );
        Part::Content(view)
    }

    pub(super) fn rehost(
        &mut self,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Chat>,
    ) {
        for reply in self.0.values() {
            reply
                .view
                .update(cx, |view, cx| view.rehost(workspace.clone(), window, cx));
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn retain(&mut self, keys: &HashSet<u64>) {
        // Releasing historical rows closes their surfaces and child workers.
        self.0.retain(|key, _| keys.contains(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn plain_rows_draw_current_text_without_retaining_reply_surfaces() {
        let mut app = ::gpui::TestAppContext::single();
        app.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        let visual = app.add_empty_window();
        let chat = visual.update(|window, cx| {
            let contributors: Vec<_> = crate::ui::extensions::catalog()
                .into_iter()
                .filter(|manifest| manifest.content.is_some())
                .map(|manifest| manifest.id)
                .collect();
            crate::ui::prefs::update_visual(cx, |prefs| {
                for id in contributors {
                    prefs.extensions.set_enabled(id, false);
                }
            });
            cx.new(|cx| {
                Chat::base(
                    None,
                    WeakEntity::new_invalid(),
                    super::super::Session::new("/synthetic/plain-replies".into()),
                    std::rc::Rc::new(|_, _, _| None),
                    window,
                    cx,
                )
            })
        });
        for version in ["initial", "updated"] {
            visual.draw(
                point(px(0.), px(0.)),
                size(px(600.), px(800.)),
                |window, cx| {
                    chat.update(cx, |chat, cx| {
                        let mut rows = div().flex().flex_col();
                        for key in 0..200 {
                            let current: Arc<str> = format!("{version} reply {key}").into();
                            let part = chat.replies.part(
                                key,
                                if key % 2 == 0 {
                                    "text/markdown"
                                } else {
                                    "text/plain"
                                },
                                &current,
                                chat.workspace.clone(),
                                chat.session.key.clone(),
                                false,
                                window,
                                cx,
                            );
                            let Part::Plain(text) = part else {
                                panic!("disabled contributors must fall back to text");
                            };
                            assert!(Arc::ptr_eq(&text, &current));
                            rows = rows.child(SharedString::from(text));
                        }
                        assert!(chat.replies.is_empty());
                        rows
                    })
                },
            );
        }
    }
}
