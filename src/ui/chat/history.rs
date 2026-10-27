//! A live tail and a bounded, bidirectional window over persisted transcript pages.
use std::ops::Range;

use super::Chat;
use crate::transcript::{Change, Transcript, window_changes};
use gpui_kit::*;
use proteus_contracts::app_protocol::{
    AppTranscriptCursor, AppTranscriptMessage, AppTranscriptPage,
};

const PAGE_ROWS: usize = 50;
const CACHE_ROWS: usize = 200;
const CACHE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Older,
    Newer,
}

#[derive(Default)]
pub(super) struct History {
    cursor: Option<AppTranscriptCursor>,
    archive: Option<HistoryWindow>,
    epoch: u64,
    loading: bool,
}

struct HistoryWindow {
    start: usize,
    transcript: Transcript,
}

impl HistoryWindow {
    fn end(&self) -> usize {
        self.start + self.transcript.entries().len()
    }

    fn trim(&mut self, direction: Direction, visible: Range<usize>) -> Option<Change> {
        let entries = self.transcript.entries();
        let mut bytes: usize = entries.iter().map(|row| row.retained_bytes()).sum();
        let mut keep = 0..entries.len();
        // A page and the visible rows are indivisible. A large row is never truncated.
        while keep.len() > CACHE_ROWS || bytes > CACHE_BYTES {
            let index = match direction {
                Direction::Older if keep.end > visible.end.max(PAGE_ROWS) => keep.end - 1,
                Direction::Newer if keep.start < visible.start && keep.len() > PAGE_ROWS => {
                    keep.start
                }
                _ => break,
            };
            bytes = bytes.saturating_sub(entries[index].retained_bytes());
            match direction {
                Direction::Older => keep.end -= 1,
                Direction::Newer => keep.start += 1,
            }
        }
        let removed = match direction {
            Direction::Older => keep.end..entries.len(),
            Direction::Newer => 0..keep.start,
        };
        if removed.is_empty() {
            return None;
        }
        self.start += removed.start.eq(&0).then_some(removed.len()).unwrap_or(0);
        Some(self.transcript.remove(removed))
    }
}

impl History {
    pub(super) fn displayed<'a>(&'a self, live: &'a Transcript) -> &'a Transcript {
        self.archive
            .as_ref()
            .map_or(live, |window| &window.transcript)
    }

    pub(super) fn archived(&self) -> bool {
        self.archive.is_some()
    }

    pub(super) fn apply(
        &mut self,
        live: &mut Transcript,
        messages: Vec<AppTranscriptMessage>,
        next: Option<AppTranscriptCursor>,
    ) -> (Change, Vec<Change>) {
        let prefix = match (&self.cursor, &next) {
            (Some(old), Some(next)) if old.revision == next.revision => Some(usize::MAX),
            (Some(old), Some(next)) => next
                .base
                .as_ref()
                .filter(|base| base.revision == old.revision)
                .map(|base| base.common_prefix),
            _ => None,
        };
        let change = live.reset(messages);
        let display = match (&mut self.archive, prefix) {
            (Some(window), Some(prefix)) if prefix > window.start => {
                if prefix < window.end() {
                    vec![
                        window
                            .transcript
                            .remove(prefix - window.start..window.transcript.entries().len()),
                    ]
                } else {
                    Vec::new()
                }
            }
            (Some(_), _) => self.latest(live),
            (None, _) => vec![change],
        };
        self.cursor = next;
        self.epoch += 1;
        self.loading = false;
        (change, display)
    }

    fn latest(&mut self, live: &Transcript) -> Vec<Change> {
        self.epoch += 1;
        self.loading = false;
        self.archive.take().map_or_else(Vec::new, |window| {
            window_changes(window.transcript.entries(), live.entries())
        })
    }

    fn request(&self, direction: Direction, live: &Transcript) -> Option<AppTranscriptCursor> {
        let mut cursor = self.cursor.clone()?;
        match direction {
            Direction::Older => {
                cursor.before = self
                    .archive
                    .as_ref()
                    .map_or(cursor.before, |window| window.start);
                if cursor.before == 0 {
                    return None;
                }
            }
            Direction::Newer => {
                let end = self.archive.as_ref()?.end();
                // The overlap with the live tail is already in memory.
                if end >= cursor.before {
                    return None;
                }
                cursor.before = (end + PAGE_ROWS).min(cursor.before + live.entries().len());
            }
        }
        Some(cursor)
    }

    fn accept_page(
        &mut self,
        live: &Transcript,
        direction: Direction,
        requested: &AppTranscriptCursor,
        page: AppTranscriptPage,
        visible: Range<usize>,
    ) -> Option<Vec<Change>> {
        if self.cursor.as_ref()?.revision != requested.revision
            || page.revision != requested.revision
            || page.start + page.messages.len() != requested.before
            || page.messages.is_empty()
        {
            return None;
        }
        let window = self.archive.get_or_insert_with(|| HistoryWindow {
            start: self.cursor.as_ref().unwrap().before,
            transcript: live.clone(),
        });
        let (change, visible) = match direction {
            Direction::Older if requested.before == window.start => {
                let count = page.messages.len();
                window.start = page.start;
                (
                    window.transcript.prepend(page.messages),
                    visible.start + count..visible.end + count,
                )
            }
            Direction::Newer if page.start == window.end() => {
                (window.transcript.append_page(page.messages, live), visible)
            }
            _ => return None,
        };
        let mut changes = vec![change];
        if let Some(change) = window.trim(direction, visible) {
            changes.push(change);
        }
        Some(changes)
    }

    fn merge_tail(&mut self, live: &Transcript, visible: Range<usize>) -> Vec<Change> {
        let Some(cursor) = &self.cursor else {
            return Vec::new();
        };
        let Some(window) = &mut self.archive else {
            return Vec::new();
        };
        let offset = window.end().saturating_sub(cursor.before);
        if offset >= live.entries().len() {
            return self.latest(live);
        }
        let mut changes = vec![window.transcript.append_known(&live.entries()[offset..])];
        if let Some(change) = window.trim(Direction::Newer, visible) {
            changes.push(change);
        }
        changes
    }
}

impl Chat {
    pub(super) fn display_transcript(&self) -> &Transcript {
        self.history.displayed(&self.session.transcript)
    }

    pub(super) fn show_latest(&mut self, cx: &mut Context<Self>) {
        let changes = self.history.latest(&self.session.transcript);
        self.apply_history_changes(changes, cx);
        self.scroller
            .update(cx, |state, cx| state.scroll_to_end(cx));
    }

    pub(super) fn apply_history_changes(&mut self, changes: Vec<Change>, cx: &mut Context<Self>) {
        for change in changes {
            self.apply_change(change, cx);
        }
        self.scroller.update(cx, |state, cx| {
            state.set_archived(self.history.archived(), cx);
        });
        let rows = self.display_transcript().entries();
        let start = self.history.archive.as_ref().map_or_else(
            || {
                self.history
                    .cursor
                    .as_ref()
                    .map_or(0, |cursor| cursor.before)
            },
            |window| window.start,
        );
        crate::startup_trace::history_window(
            start,
            rows.len(),
            rows.iter().map(|row| row.retained_bytes()).sum(),
            self.session.transcript.entries().len(),
        );
        cx.notify();
    }

    pub(super) fn load_history(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if self.history.loading {
            return;
        }
        let Some(cursor) = self.history.request(direction, &self.session.transcript) else {
            if direction == Direction::Newer && self.history.archived() {
                let visible = self.scroller.read(cx).visible_rows();
                let changes = self.history.merge_tail(&self.session.transcript, visible);
                self.apply_history_changes(changes, cx);
            }
            return;
        };
        self.history.loading = true;
        let epoch = self.history.epoch;
        let api = self.api.clone();
        let dir = self.session.dir.clone();
        cx.spawn(async move |this, cx| {
            let requested = cursor.clone();
            let page = cx
                .background_executor()
                .spawn(async move { api.history_page(&dir, &requested) })
                .await;
            this.update(cx, |this, cx| {
                if this.history.epoch != epoch {
                    return;
                }
                this.history.loading = false;
                match page {
                    Ok(page) => {
                        let visible = this.scroller.read(cx).visible_rows();
                        if let Some(changes) = this.history.accept_page(
                            &this.session.transcript,
                            direction,
                            &cursor,
                            page,
                            visible,
                        ) {
                            this.apply_history_changes(changes, cx);
                        } else {
                            this.show_notice("История изменилась; ожидаю новый снимок сессии", cx);
                        }
                    }
                    Err(error) => {
                        this.show_notice(format!("Не удалось загрузить историю: {error:#}"), cx)
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests;
