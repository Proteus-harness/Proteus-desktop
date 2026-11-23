//! Codex history keeps only a bounded window. Cursor metadata contains no text.
use super::super::history::Direction;
use super::*;
use crate::transcript::Entry;

const ROWS: usize = 200;
const BYTES: usize = 8 * 1024 * 1024;

fn retained_bytes(entries: &[Entry]) -> usize {
    entries.iter().map(Entry::retained_bytes).sum()
}

impl Chat {
    pub(in crate::ui::chat) fn pause_codex_before_eviction(
        &mut self,
        event: &Event,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = &mut self.codex else {
            return;
        };
        if binding.archive.is_some()
            || self.session.transcript.entries().is_empty()
            || !self.scroller.read(cx).reading_paused()
        {
            return;
        }
        let incoming = match event {
            Event::ItemUpsert(item) => Some((&item.turn_id, &item.id)),
            Event::TextDelta { id, turn_id, .. }
            | Event::ReasoningDelta { id, turn_id, .. }
            | Event::ToolOutputDelta { id, turn_id, .. } => Some((turn_id, id)),
            _ => None,
        };
        if incoming.is_none_or(|(turn, id)| self.session.transcript.external_contains(turn, id)) {
            return;
        }
        binding.archive = Some(self.session.transcript.clone());
        binding
            .page_rows
            .push_back(self.session.transcript.entries().len());
        // No list splice occurs: these are the exact rows the reader saw.
        // Future live eviction changes only the separate bounded tail.
        self.scroller
            .update(cx, |state, cx| state.set_archived(true, cx));
    }

    pub(in crate::ui::chat) fn codex_latest(&mut self, cx: &mut Context<Self>) {
        let binding = self.codex.as_mut().unwrap();
        binding.history_epoch = binding.history_epoch.wrapping_add(1);
        binding
            .history_cancel
            .store(true, std::sync::atomic::Ordering::Release);
        binding.history_loading = false;
        binding.history_exhausted = false;
        let old = binding.archive.take();
        binding.cursor_chain = vec![None];
        binding.page_rows.clear();
        binding.oldest_page = 0;
        binding.newest_page = 0;
        if let Some(old) = old {
            for change in
                crate::transcript::window_changes(old.entries(), self.session.transcript.entries())
            {
                self.apply_change(change, cx);
            }
        }
        self.scroller.update(cx, |state, cx| {
            state.set_archived(false, cx);
            state.history_ready(cx);
            state.scroll_to_end(cx);
        });
    }

    pub(in crate::ui::chat) fn codex_history(
        &mut self,
        direction: Direction,
        cx: &mut Context<Self>,
    ) {
        let binding = self.codex.as_mut().unwrap();
        if binding.history_loading
            || !self.session.connected
            || (direction == Direction::Older && binding.history_exhausted)
        {
            return;
        }
        if direction == Direction::Newer && binding.newest_page == 0 {
            if binding.archive.is_some() {
                self.codex_latest(cx);
            }
            return;
        }
        let fresh = binding.cursor_chain.len() == 1;
        let anchor = binding
            .archive
            .as_ref()
            .unwrap_or(&self.session.transcript)
            .external_first_identity();
        let index = match direction {
            Direction::Older => binding.oldest_page + 1,
            Direction::Newer => binding.newest_page - 1,
        };
        let cursor = binding
            .cursor_chain
            .get(index)
            .cloned()
            .flatten()
            .or_else(|| {
                (direction == Direction::Older)
                    .then(|| binding.next_cursor.clone())
                    .flatten()
            });
        if !fresh && index > 0 && cursor.is_none() {
            return;
        }
        binding.history_loading = true;
        binding.history_epoch += 1;
        binding
            .history_cancel
            .store(true, std::sync::atomic::Ordering::Release);
        binding.history_cancel = Default::default();
        let cancel = binding.history_cancel.clone();
        let epoch = binding.history_epoch;
        let handle = binding.handle.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    // Resolve the frozen reader's actual oldest opaque item.
                    // The live tail may have moved several pages meanwhile.
                    // Each page is discarded before reading the next one.
                    let mut cursor = cursor;
                    if fresh {
                        cursor = None;
                        loop {
                            if cancel.load(std::sync::atomic::Ordering::Acquire) { return Err(crate::agents::codex::Error::Protocol("History request canceled".into())); }
                            let mut page = handle.history(cursor.as_deref())?;
                            let boundary = anchor.as_ref().and_then(|(turn, id)| page.data.iter().position(|item| &item.turn_id == turn && &item.id == id));
                            if let Some(boundary) = boundary {
                                page.data.truncate(boundary);
                                if !page.data.is_empty() { return Ok((Some(page), cursor)); }
                                cursor = page.next_cursor;
                                break;
                            }
                            if page.next_cursor.is_none() {
                                if anchor.is_some() { return Err(crate::agents::codex::Error::Protocol("Reading anchor is no longer in Codex history; use the latest-message button".into())); }
                                return Ok((None, None));
                            }
                            if page.next_cursor == cursor { return Err(crate::agents::codex::Error::Protocol("Codex history cursor did not advance".into())); }
                            cursor = page.next_cursor;
                        }
                    }
                    if index > 0 && cursor.is_none() { return Ok((None, cursor)); }
                    if cancel.load(std::sync::atomic::Ordering::Acquire) { return Err(crate::agents::codex::Error::Protocol("History request canceled".into())); }
                    handle
                        .history(cursor.as_deref())
                        .map(|page| (Some(page), cursor))
                })
                .await;
            this.update(cx, |this, cx| {
                let binding = this.codex.as_mut().unwrap();
                if binding.history_epoch != epoch {
                    return;
                }
                binding.history_loading = false;
                match result {
                    Ok((Some(page), requested)) => {
                        if binding.archive.is_none() {
                            binding.archive = Some(this.session.transcript.clone());
                            binding
                                .page_rows
                                .push_back(this.session.transcript.entries().len());
                        }
                        while binding.cursor_chain.len() <= index {
                            binding.cursor_chain.push(None);
                        }
                        binding.cursor_chain[index] = requested;
                        if direction == Direction::Older {
                            binding.next_cursor = page.next_cursor.clone();
                            if binding.cursor_chain.len() == index + 1 {
                                binding.cursor_chain.push(page.next_cursor);
                            }
                        }
                        let archive = binding.archive.as_mut().unwrap();
                        let before = archive.entries().len();
                        let mut changes = Vec::new();
                        match direction {
                            Direction::Older => {
                                changes.push(archive.external_prepend(page.data));
                                let inserted = archive.entries().len() - before;
                                binding.page_rows.push_front(inserted);
                                binding.oldest_page = index;
                            }
                            Direction::Newer => {
                                for item in page.data {
                                    changes.push(archive.external_upsert(item));
                                }
                                binding
                                    .page_rows
                                    .push_back(archive.entries().len() - before);
                                binding.newest_page = index;
                            }
                        }
                        let mut visible = this.scroller.read(cx).visible_rows();
                        if direction == Direction::Older {
                            let inserted = archive.entries().len() - before;
                            visible = visible.start + inserted..visible.end + inserted;
                        }
                        let mut retained = None;
                        while binding.page_rows.len() > 1 {
                            let entries = archive.entries();
                            let (count, range) = match direction {
                                Direction::Older => {
                                    let count = *binding.page_rows.back().unwrap();
                                    (
                                        count,
                                        archive.entries().len() - count..archive.entries().len(),
                                    )
                                }
                                Direction::Newer => {
                                    let count = *binding.page_rows.front().unwrap();
                                    (count, 0..count)
                                }
                            };
                            if range.start < visible.end && range.end > visible.start {
                                break;
                            }
                            // The row limit takes precedence. Once accounting
                            // starts, rows only leave the window by whole pages.
                            let counted = if entries.len() > ROWS {
                                None
                            } else {
                                let (total, edge) = match retained {
                                    Some(total) => (total, None),
                                    None if binding.page_rows.len() == 2 => {
                                        (retained_bytes(entries), None)
                                    }
                                    None => {
                                        // Count the first evictee during the
                                        // initial scan, not again after a breach.
                                        let split = match direction {
                                            Direction::Older => range.start,
                                            Direction::Newer => range.end,
                                        };
                                        let (left, right) = entries.split_at(split);
                                        let left = retained_bytes(left);
                                        let right = retained_bytes(right);
                                        let edge = match direction {
                                            Direction::Older => right,
                                            Direction::Newer => left,
                                        };
                                        (left + right, Some(edge))
                                    }
                                };
                                if total <= BYTES {
                                    break;
                                }
                                Some((total, edge))
                            };
                            if binding.page_rows.len() > 2 {
                                retained = counted.map(|(total, edge)| {
                                    // Plain usize sums wrap in release. Modular
                                    // subtraction gives the same survivor sum.
                                    total.wrapping_sub(edge.unwrap_or_else(|| {
                                        retained_bytes(&entries[range.clone()])
                                    }))
                                });
                            }
                            changes.push(archive.remove(range));
                            match direction {
                                Direction::Older => {
                                    binding.page_rows.pop_back();
                                    binding.newest_page += 1;
                                }
                                Direction::Newer => {
                                    binding.page_rows.pop_front();
                                    binding.oldest_page = binding.oldest_page.saturating_sub(1);
                                    visible = visible.start.saturating_sub(count)
                                        ..visible.end.saturating_sub(count);
                                }
                            }
                        }
                        archive.external_prune();
                        for change in changes {
                            this.apply_change(change, cx);
                        }
                        this.scroller.update(cx, |state, cx| {
                            state.set_archived(true, cx);
                            state.history_ready(cx);
                        });
                    }
                    Ok((None, _)) => {
                        binding.history_exhausted = true;
                        this.scroller
                            .update(cx, |state, cx| state.history_ready(cx));
                    }
                    Err(error) => {
                        this.show_notice(
                            format!("Не удалось загрузить историю Codex: {error}"),
                            cx,
                        );
                        // A fresh wheel gesture retries. Do not repeatedly
                        // issue a failed request from stationary prepaint.
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
