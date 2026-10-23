//! Reply blocks retained for the chat's lifetime, across virtualized scroll
//! and background tabs. Completed registered fences get independent native
//! surfaces; ordinary and unfinished markdown keeps its usual text renderer.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::text::TextViewState;
use gpui_kit::*;

use super::{Chat, fences};
use crate::declarative::protocol::Content;
use crate::ui::extensions::{Host, content};
use crate::ui::workspace::Workspace;

const BACKGROUND_PARSE_BYTES: usize = 4 * 1024;

#[derive(Default)]
pub(super) struct Replies(HashMap<u64, Reply>);

#[derive(Clone)]
pub(super) enum Part {
    Markdown(Entity<TextViewState>),
    Content(Entity<content::ContentView>),
}

struct Reply {
    text: Arc<str>,
    languages: Vec<(String, &'static str)>,
    pieces: Vec<Piece>,
    fences: Vec<fences::Fence>,
}

struct Piece {
    start: usize,
    block: Option<usize>,
    text: Arc<str>,
    /// The contributor is part of identity: disabling it restores raw text.
    renderer: Option<&'static str>,
    part: Part,
    _changes: Subscription,
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
        let languages = content::languages(cx);
        if let Some(reply) = self.0.get(&key)
            && Arc::ptr_eq(&reply.text, text)
            && reply.languages == languages
        {
            return reply
                .pieces
                .iter()
                .map(|piece| piece.part.clone())
                .collect();
        }
        let found = match self.0.get(&key) {
            Some(reply) if reply.languages == languages => {
                fences::appended(text, &reply.text, &reply.fences, &languages)
            }
            _ => fences::registered(text, &languages),
        };
        let mut old = self
            .0
            .remove(&key)
            .map_or_else(Vec::new, |reply| reply.pieces);
        let mut pieces = Vec::new();
        let mut start = 0;
        for fence in &found {
            if start < fence.range.start {
                pieces.push(markdown_piece(
                    &mut old,
                    key,
                    start,
                    &text[start..fence.range.start],
                    cx,
                ));
            }
            let raw = &text[fence.range.clone()];
            let old_index = old.iter().position(|piece| {
                piece.block == Some(fence.ordinal)
                    && piece.renderer == Some(fence.extension)
                    && piece.text.as_ref() == raw
            });
            let piece = if let Some(index) = old_index {
                let mut piece = old.remove(index);
                piece.start = fence.range.start;
                piece
            } else {
                let raw: Arc<str> = raw.into();
                let payload = Content {
                    language: fence.language.clone(),
                    source: text[fence.source.clone()].into(),
                };
                let host = Host::new(workspace.clone(), fence.extension)
                    .for_chat(cx.weak_entity(), session.clone());
                // A changed source is a new immutable view, so a late reply
                // to the previous source cannot overwrite the replacement.
                let id = format!("{key}-{}-{}", fence.ordinal, fingerprint(&raw));
                let view = cx.new(|cx| {
                    content::ContentView::new(host, id, payload, raw.clone(), window, cx)
                });
                let resize = cx.subscribe(&view, move |chat, _, _: &content::WillResize, cx| {
                    if let Some(row) = chat.row_of(key) {
                        chat.scroller
                            .update(cx, |scroller, cx| scroller.toggled(row..row + 1, cx));
                    }
                });
                Piece {
                    start: fence.range.start,
                    block: Some(fence.ordinal),
                    text: raw,
                    renderer: Some(fence.extension),
                    _changes: cx.observe(&view, move |chat, _, cx| chat.remeasure_row(key, cx)),
                    _resize: Some(resize),
                    part: Part::Content(view),
                }
            };
            pieces.push(piece);
            start = fence.range.end;
        }
        if start < text.len() || pieces.is_empty() {
            pieces.push(markdown_piece(&mut old, key, start, &text[start..], cx));
        }
        let result = pieces.iter().map(|piece| piece.part.clone()).collect();
        self.0.insert(
            key,
            Reply {
                text: text.clone(),
                languages,
                pieces,
                fences: found,
            },
        );
        result
    }

    pub(super) fn prepare<'a>(
        &mut self,
        replies: impl IntoIterator<Item = (u64, &'a Arc<str>)>,
        cx: &mut Context<Chat>,
    ) {
        let languages = content::languages(cx);
        for (key, text) in replies {
            if text.len() > BACKGROUND_PARSE_BYTES
                && !self.0.contains_key(&key)
                && fences::registered(text, &languages).is_empty()
            {
                let piece = markdown_piece(&mut Vec::new(), key, 0, text, cx);
                self.0.insert(
                    key,
                    Reply {
                        text: text.clone(),
                        languages: languages.clone(),
                        pieces: vec![piece],
                        fences: Vec::new(),
                    },
                );
            }
        }
    }

    pub(super) fn retain(&mut self, keys: &HashSet<u64>) {
        // Dropping a content view sends close and releases its process when
        // the chat has no remaining views belonging to that package.
        self.0.retain(|key, _| keys.contains(key));
    }
}

fn markdown_piece(
    old: &mut Vec<Piece>,
    key: u64,
    start: usize,
    text: &str,
    cx: &mut Context<Chat>,
) -> Piece {
    if let Some(index) = old
        .iter()
        .position(|piece| piece.start == start && piece.renderer.is_none())
    {
        let mut piece = old.remove(index);
        if piece.text.as_ref() != text {
            piece.text = text.into();
            if let Part::Markdown(state) = &piece.part {
                state.update(cx, |state, cx| state.set_text(text, cx));
            }
        }
        return piece;
    }
    let state = cx.new(|cx| TextViewState::markdown(text, cx));
    Piece {
        start,
        block: None,
        text: text.into(),
        renderer: None,
        _changes: cx.observe(&state, move |chat, _, cx| chat.remeasure_row(key, cx)),
        _resize: None,
        part: Part::Markdown(state),
    }
}

fn fingerprint(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
