//! Click-time copy reads a whole response without expanding the visible cache.
use std::{collections::HashSet, path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow, bail, ensure};
use proteus_contracts::app_protocol::{AppTranscriptCursor, AppTranscriptPage};

use super::Chat;
use crate::{
    agents::codex::{Item, ItemKind, Page, ThreadHandle},
    api::Api,
    transcript::Role,
};

const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_PAGES: usize = 4096;

pub(super) struct Plan {
    parts: Parts,
    older: Option<Source>,
}

enum Source {
    Core {
        api: Api,
        session: PathBuf,
        cursor: AppTranscriptCursor,
    },
    Codex {
        handle: ThreadHandle,
        turn: String,
        anchor: Anchor,
        cursor: Option<String>,
        seen: HashSet<String>,
    },
}

struct Anchor {
    turn: String,
    item: String,
    role: Role,
    text: Arc<str>,
}

#[derive(Default)]
struct Parts {
    // Walk newest to oldest; reverse only once, when producing the clipboard.
    newest_first: Vec<Arc<str>>,
    bytes: usize,
}

impl Parts {
    fn length_with(&self, length: usize) -> Result<usize> {
        let separators = usize::from(!self.newest_first.is_empty()) * 2;
        let bytes = self
            .bytes
            .checked_add(length)
            .and_then(|n| n.checked_add(separators));
        ensure!(
            bytes.is_some_and(|n| n <= MAX_BYTES),
            "Текст хода превышает лимит копирования 16 МиБ"
        );
        Ok(bytes.unwrap())
    }

    fn push(&mut self, text: Arc<str>) -> Result<()> {
        if text.trim().is_empty() {
            return Ok(());
        }
        self.bytes = self.length_with(text.len())?;
        self.newest_first.push(text);
        Ok(())
    }

    fn push_owned(&mut self, text: String) -> Result<()> {
        if text.trim().is_empty() {
            return Ok(());
        }
        // Reject oversized source before allocating the shared text fragment.
        self.length_with(text.len())?;
        self.push(text.into())
    }

    fn finish(self) -> Result<String> {
        ensure!(
            !self.newest_first.is_empty(),
            "В этом ходе нет текста ответа для копирования"
        );
        let mut text = String::with_capacity(self.bytes);
        for part in self.newest_first.into_iter().rev() {
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str(&part);
        }
        Ok(text)
    }
}

impl Chat {
    pub(super) fn turn_copy_plan(&self, end_key: u64) -> Result<Plan> {
        let transcript = self.display_transcript();
        let entries = transcript.entries();
        let end = entries
            .iter()
            .position(|entry| entry.key == end_key)
            .ok_or_else(|| anyhow!("Этот ход больше не находится в открытой истории"))?;
        ensure!(
            entries[end].role != Role::User,
            "У этого сообщения ещё нет ответа"
        );
        let codex_turn = self
            .codex
            .as_ref()
            .map(|_| {
                transcript
                    .external_identity(end_key)
                    .map(|(turn, _)| turn.to_owned())
                    .ok_or_else(|| anyhow!("Не удалось определить ход Codex"))
            })
            .transpose()?;
        let mut parts = Parts::default();
        let mut complete = false;
        for entry in entries[..=end].iter().rev() {
            if entry.role == Role::User {
                complete = true;
                break;
            }
            if let Some(turn) = &codex_turn {
                let (row_turn, _) = transcript
                    .external_identity(entry.key)
                    .ok_or_else(|| anyhow!("Не удалось определить границу хода Codex"))?;
                if row_turn != turn {
                    complete = true;
                    break;
                }
            }
            ensure!(
                !entry.streaming,
                "Ответ ещё изменяется; дождитесь завершения хода"
            );
            if entry.role == Role::Assistant {
                parts.push(entry.text.clone())?;
            }
        }
        let older = if complete {
            None
        } else if let Some(binding) = &self.codex {
            let first = &entries[0];
            let (turn, item) = transcript
                .external_identity(first.key)
                .ok_or_else(|| anyhow!("Не удалось определить начало истории Codex"))?;
            // Archived windows trim whole pages, so this cursor still addresses
            // their oldest page. A moving live tail is resolved by opaque anchor.
            let cursor = binding.archive.as_ref().and_then(|_| {
                binding
                    .cursor_chain
                    .get(binding.oldest_page)
                    .cloned()
                    .flatten()
            });
            Some(Source::Codex {
                handle: binding.handle.clone(),
                turn: codex_turn.unwrap(),
                anchor: Anchor {
                    turn: turn.to_owned(),
                    item: item.to_owned(),
                    role: first.role,
                    text: first.text.clone(),
                },
                cursor,
                seen: entries[..=end]
                    .iter()
                    .filter_map(|entry| {
                        transcript
                            .external_identity(entry.key)
                            .map(|(_, item)| item.to_owned())
                    })
                    .collect(),
            })
        } else {
            self.history
                .copy_cursor(&self.session.transcript)
                .map(|cursor| -> Result<Source> {
                    Ok(Source::Core {
                        api: self
                            .api
                            .clone()
                            .ok_or_else(|| anyhow!("Сервер истории недоступен"))?,
                        session: self.session.dir.clone(),
                        cursor,
                    })
                })
                .transpose()?
        };
        Ok(Plan { parts, older })
    }
}

impl Plan {
    /// Blocking, independent read. Call from the background executor.
    pub(super) fn read(mut self) -> Result<String> {
        match self.older {
            Some(Source::Core {
                api,
                session,
                cursor,
            }) => {
                read_core(&mut self.parts, cursor, |cursor| {
                    api.history_page(&session, cursor)
                })?;
            }
            Some(Source::Codex {
                handle,
                turn,
                anchor,
                cursor,
                seen,
            }) => {
                read_codex(&mut self.parts, &turn, anchor, cursor, seen, |cursor| {
                    handle.history(cursor).map_err(Into::into)
                })?;
            }
            None => {}
        }
        self.parts.finish()
    }
}

fn read_core(
    parts: &mut Parts,
    mut cursor: AppTranscriptCursor,
    mut read: impl FnMut(&AppTranscriptCursor) -> Result<AppTranscriptPage>,
) -> Result<()> {
    for _ in 0..MAX_PAGES {
        if cursor.before == 0 {
            return Ok(());
        }
        let page = read(&cursor)?;
        ensure!(
            page.revision == cursor.revision
                && !page.messages.is_empty()
                && page.start.checked_add(page.messages.len()) == Some(cursor.before)
                && page.total >= cursor.before
                && page.start < cursor.before,
            "История изменилась или страница хода недоступна; повторите копирование"
        );
        for message in page.messages.into_iter().rev() {
            if message.role == "user" {
                return Ok(());
            }
            ensure!(
                !message.streaming,
                "Ответ в сохранённой истории ещё изменяется"
            );
            if message.role == "assistant" {
                parts.push_owned(message.text)?;
            }
        }
        cursor.before = page.start;
        if cursor.before == 0 {
            return Ok(());
        }
    }
    bail!("Ход превышает лимит страниц для копирования")
}

fn read_codex(
    parts: &mut Parts,
    turn: &str,
    anchor: Anchor,
    mut cursor: Option<String>,
    mut items: HashSet<String>,
    mut read: impl FnMut(Option<&str>) -> Result<Page<Item>>,
) -> Result<()> {
    let mut cursors = HashSet::new();
    let mut anchored = false;
    for _ in 0..MAX_PAGES {
        ensure!(
            cursors.insert(cursor.clone()),
            "Курсор истории Codex не продвигается"
        );
        let page = read(cursor.as_deref())?;
        let end = if anchored {
            page.data.len()
        } else if let Some(index) = page
            .data
            .iter()
            .position(|item| item.turn_id == anchor.turn && item.id == anchor.item)
        {
            let item = &page.data[index];
            if anchor.role == Role::Assistant {
                let text = match &item.kind {
                    ItemKind::Assistant { text } | ItemKind::Plan { text } => text.as_str(),
                    _ => bail!("Начало хода Codex изменилось; повторите копирование"),
                };
                ensure!(
                    text == anchor.text.as_ref(),
                    "Текст хода Codex изменился; повторите копирование"
                );
            }
            anchored = true;
            index
        } else {
            if page.next_cursor.is_none() {
                bail!("Начало открытого хода больше недоступно в истории Codex");
            }
            cursor = page.next_cursor;
            continue;
        };
        for item in page.data.into_iter().take(end).rev() {
            if item.turn_id != turn || matches!(&item.kind, ItemKind::User { .. }) {
                return Ok(());
            }
            ensure!(
                items.insert(item.id.clone()),
                "Страницы истории Codex перекрываются"
            );
            if let ItemKind::Assistant { text } | ItemKind::Plan { text } = item.kind {
                ensure!(
                    !matches!(item.status.as_str(), "inProgress" | "running"),
                    "Ответ Codex ещё изменяется"
                );
                parts.push_owned(text)?;
            }
        }
        let Some(next) = page.next_cursor else {
            return Ok(());
        };
        cursor = Some(next);
    }
    bail!("Ход Codex превышает лимит страниц для копирования")
}

#[cfg(test)]
mod tests;
