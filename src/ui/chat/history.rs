//! Older rows are read on demand; a snapshot retains only a server-verified prefix.
use super::Chat;
use crate::transcript::{Change, Transcript};
use gpui_kit::*;
use proteus_contracts::app_protocol::{AppTranscriptCursor, AppTranscriptMessage};

#[derive(Default)]
pub(super) struct History {
    cursor: Option<AppTranscriptCursor>,
    epoch: u64,
    loading: bool,
}

impl History {
    pub(super) fn apply(
        &mut self,
        transcript: &mut Transcript,
        messages: Vec<AppTranscriptMessage>,
        next: Option<AppTranscriptCursor>,
    ) -> Change {
        let keep = match (&self.cursor, &next) {
            (Some(old), Some(next))
                if old.before <= next.before
                    && (old.revision == next.revision
                        || next.base.as_ref().is_some_and(|base| {
                            base.revision == old.revision && base.common_prefix >= next.before
                        })) =>
            {
                next.before - old.before
            }
            _ => 0,
        }
        .min(transcript.entries().len());
        let before = if keep > 0 {
            self.cursor.as_ref().unwrap().before
        } else {
            next.as_ref().map_or(0, |cursor| cursor.before)
        };
        self.cursor = next.map(|cursor| AppTranscriptCursor { before, ..cursor });
        self.epoch += 1;
        self.loading = false;
        transcript.reset_suffix(keep, messages)
    }
}

impl Chat {
    pub(super) fn load_older(&mut self, cx: &mut Context<Self>) {
        if self.history.loading {
            return;
        }
        let Some(cursor) = self.history.cursor.clone() else {
            return;
        };
        if cursor.before == 0 {
            return;
        }
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
                if this.history.epoch != epoch || this.history.cursor.as_ref() != Some(&cursor) {
                    return;
                }
                this.history.loading = false;
                match page {
                    Ok(page)
                        if page.revision == cursor.revision
                            && page.start + page.messages.len() == cursor.before =>
                    {
                        this.history.cursor = Some(AppTranscriptCursor {
                            before: page.start,
                            ..cursor
                        });
                        let change = this.session.transcript.prepend(page.messages);
                        this.apply_change(change, cx);
                        cx.emit(super::ChatEvent::SessionEntriesChanged(change));
                    }
                    Ok(_) => {
                        this.show_notice("История изменилась; ожидаю новый снимок сессии", cx);
                    }
                    Err(error) => {
                        this.show_notice(format!("Не удалось загрузить историю: {error:#}"), cx);
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
mod tests {
    use super::History;
    use crate::transcript::Transcript;
    use proteus_contracts::app_protocol::{
        AppTranscriptBase, AppTranscriptCursor, AppTranscriptMessage,
    };

    fn rows(start: usize, end: usize) -> Vec<AppTranscriptMessage> {
        (start..end)
            .map(|i| AppTranscriptMessage {
                message_id: Some(uuid::Uuid::from_u128(i as u128 + 1)),
                phase: None,
                role: "user".into(),
                text: format!("row {i}"),
                images: vec![],
                tool: None,
                subagent: None,
                streaming: false,
            })
            .collect()
    }

    fn cursor(revision: &str, before: usize, base: Option<(&str, usize)>) -> AppTranscriptCursor {
        AppTranscriptCursor {
            revision: revision.into(),
            before,
            base: base.map(|(revision, common_prefix)| AppTranscriptBase {
                revision: revision.into(),
                common_prefix,
            }),
        }
    }

    #[test]
    fn loaded_prefix_and_row_identity_survive_reconnect_and_verified_append() {
        let mut state = History::default();
        let mut transcript = Transcript::default();
        state.apply(
            &mut transcript,
            rows(100, 150),
            Some(cursor("first", 100, None)),
        );
        transcript.prepend(rows(50, 100));
        state.cursor.as_mut().unwrap().before = 50;
        let key = transcript.entries()[0].key;
        state.apply(
            &mut transcript,
            rows(100, 150),
            Some(cursor("first", 100, None)),
        );
        assert_eq!(transcript.entries().len(), 100);
        assert_eq!(transcript.entries()[0].key, key);
        state.apply(
            &mut transcript,
            rows(102, 152),
            Some(cursor("second", 102, Some(("first", 150)))),
        );
        assert_eq!(transcript.entries().len(), 102);
        assert_eq!(transcript.entries()[0].key, key);
        assert_eq!(&*transcript.entries()[101].text, "row 151");
        assert_eq!(state.cursor.unwrap().before, 50);
    }

    #[test]
    fn fully_loaded_history_stays_loaded_but_unverified_rewrite_discards_old_rows() {
        let mut state = History::default();
        let mut transcript = Transcript::default();
        state.apply(
            &mut transcript,
            rows(50, 100),
            Some(cursor("first", 50, None)),
        );
        transcript.prepend(rows(0, 50));
        state.cursor.as_mut().unwrap().before = 0;
        state.apply(
            &mut transcript,
            rows(52, 102),
            Some(cursor("second", 52, Some(("first", 100)))),
        );
        assert_eq!(transcript.entries().len(), 102);
        assert_eq!(state.cursor.as_ref().unwrap().before, 0);
        state.apply(
            &mut transcript,
            rows(52, 102),
            Some(cursor("rewritten", 52, Some(("second", 3)))),
        );
        assert_eq!(transcript.entries().len(), 50);
        assert_eq!(&*transcript.entries()[0].text, "row 52");
        state.apply(
            &mut transcript,
            Vec::new(),
            Some(cursor("cleared", 0, Some(("rewritten", 0)))),
        );
        assert!(transcript.entries().is_empty());
        assert_eq!(state.cursor.unwrap().before, 0);
    }
}
