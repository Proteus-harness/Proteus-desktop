//! Bounded semantic context for an explicit fractional history seek.
use super::{
    CACHE_BYTES, CACHE_ROWS, Chat, Direction, History, HistoryRange, ScrollTarget, Transcript,
    seek_context,
};
use gpui_kit::*;
use std::ops::Range;

pub(super) struct ContextSeek {
    target: ScrollTarget,
    rows: usize,
    bytes: usize,
}

#[derive(Debug, PartialEq)]
pub(super) enum ContextStep {
    Complete,
    Load(Direction),
    RowStart(ScrollTarget),
}

impl ContextSeek {
    pub(super) fn new(target: ScrollTarget) -> Self {
        Self {
            target,
            rows: 0,
            bytes: 0,
        }
    }

    pub(super) fn loaded(&mut self, entries: &[crate::transcript::Entry]) {
        self.rows += entries.len();
        self.bytes += entries
            .iter()
            .map(|entry| entry.retained_bytes())
            .sum::<usize>();
    }

    fn step(
        &self,
        entries: &[crate::transcript::Entry],
        range: HistoryRange,
        compact: bool,
    ) -> ContextStep {
        let Some(row) = self
            .target
            .row
            .checked_sub(range.start)
            .filter(|row| *row < range.rows)
        else {
            return ContextStep::RowStart(ScrollTarget {
                fraction: 0.,
                ..self.target
            });
        };
        let context = super::super::row_layout::row_context(entries, compact, Some(range));
        let Some(direction) = seek_context(range, self.target).or_else(|| context.missing(row))
        else {
            return ContextStep::Complete;
        };
        if self.rows >= CACHE_ROWS || self.bytes >= CACHE_BYTES {
            ContextStep::RowStart(ScrollTarget {
                fraction: 0.,
                ..self.target
            })
        } else {
            ContextStep::Load(direction)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::app_protocol::{
        AppTranscriptCursor, AppTranscriptMessage, AppTranscriptPage, AppTranscriptTool,
    };

    fn corpus(tools: Range<usize>) -> Vec<AppTranscriptMessage> {
        (0..600)
            .map(|row| AppTranscriptMessage {
                timestamp_ms: None,
                message_id: Some(uuid::Uuid::from_u128(row as u128 + 1)),
                phase: None,
                role: if tools.contains(&row) {
                    "system"
                } else {
                    "user"
                }
                .into(),
                text: "synthetic context".into(),
                images: Vec::new(),
                tool: tools.contains(&row).then(|| AppTranscriptTool {
                    call_id: format!("context-{row}"),
                    name: "read_file".into(),
                    args: serde_json::Value::Null,
                    effective_args: None,
                    status: "done".into(),
                    result: Some("synthetic".into()),
                    metadata: serde_json::Value::Null,
                    terminal: None,
                }),
                subagent: None,
                streaming: false,
            })
            .collect()
    }

    fn fixture(messages: &[AppTranscriptMessage], row: usize) -> (History, Transcript) {
        let mut history = History::default();
        let mut live = Transcript::default();
        history.apply(
            &mut live,
            messages[550..].to_vec(),
            Some(AppTranscriptCursor {
                revision: "context".into(),
                before: 550,
                base: None,
            }),
        );
        let target = ScrollTarget { row, fraction: 0.5 };
        history.seek = Some(target);
        let cursor = history.seek_cursor().unwrap();
        let start = cursor.before - 50;
        history
            .accept_seek(
                &live,
                &cursor,
                AppTranscriptPage {
                    revision: cursor.revision.clone(),
                    start,
                    total: messages.len(),
                    messages: messages[start..cursor.before].to_vec(),
                    base: None,
                },
            )
            .unwrap();
        history.context_seek = Some(ContextSeek::new(target));
        (history, live)
    }

    fn resolve(
        messages: &[AppTranscriptMessage],
        row: usize,
    ) -> (ContextStep, usize, History, Transcript) {
        let (mut history, live) = fixture(messages, row);
        let mut requests = 0;
        loop {
            let step = history.context_step(&live, true).unwrap();
            let ContextStep::Load(direction) = step else {
                history.context_seek = None;
                return (step, requests, history, live);
            };
            requests += 1;
            assert!(
                requests <= 4,
                "context resolution must terminate within its page budget"
            );
            let cursor = history.request(direction, &live).unwrap();
            let start = cursor.before - 50;
            let local = row - history.range(&live).unwrap().start;
            history
                .accept_page(
                    &live,
                    direction,
                    &cursor,
                    AppTranscriptPage {
                        revision: cursor.revision.clone(),
                        start,
                        total: messages.len(),
                        messages: messages[start..cursor.before].to_vec(),
                        base: None,
                    },
                    local..local + 1,
                )
                .unwrap();
            let entries = history.displayed(&live).entries();
            assert!(entries.len() <= CACHE_ROWS);
            assert!(
                entries
                    .iter()
                    .map(|entry| entry.retained_bytes())
                    .sum::<usize>()
                    <= CACHE_BYTES
            );
        }
    }

    #[::core::prelude::v1::test]
    fn a_300_tool_run_stops_at_row_start_after_four_context_pages_without_changing_cache_limits() {
        let messages = corpus(200..500);
        let (step, requests, history, live) = resolve(&messages, 350);
        assert_eq!(requests, 4);
        assert_eq!(
            step,
            ContextStep::RowStart(ScrollTarget {
                row: 350,
                fraction: 0.
            })
        );
        assert!(history.context_seek.is_none());
        let range = history.range(&live).unwrap();
        assert!(
            super::super::super::row_layout::row_context(
                history.displayed(&live).entries(),
                true,
                Some(range)
            )
            .missing(350 - range.start)
            .is_some(),
            "the fallback must not invent complete group context"
        );
    }

    #[::core::prelude::v1::test]
    fn a_smaller_multi_page_group_resolves_the_exact_fraction_before_the_budget() {
        let (step, requests, history, _) = resolve(&corpus(200..320), 270);
        assert_eq!(requests, 2);
        assert_eq!(step, ContextStep::Complete);
        assert!(history.context_seek.is_none());
    }

    #[::core::prelude::v1::test]
    fn context_budget_counts_extra_bytes_and_cancellation_clears_the_pending_request() {
        let messages = corpus(200..500);
        let (mut history, mut live) = fixture(&messages, 350);
        let mut huge = Transcript::default();
        let mut message = messages[300].clone();
        message.tool.as_mut().unwrap().result = Some("x".repeat(CACHE_BYTES));
        huge.reset(vec![message]);
        history
            .context_seek
            .as_mut()
            .unwrap()
            .loaded(huge.entries());
        assert_eq!(
            history.context_step(&live, true),
            Some(ContextStep::RowStart(ScrollTarget {
                row: 350,
                fraction: 0.
            }))
        );
        history.loading = true;
        assert!(history.request_pending());
        assert!(history.cancel_seek());
        assert!(history.context_seek.is_none());
        assert!(!history.loading);
        assert!(!history.cancel_seek());
        history.context_seek = Some(ContextSeek::new(ScrollTarget {
            row: 351,
            fraction: 0.5,
        }));
        assert!(
            history.request_pending(),
            "context resolution also owns pending navigation"
        );
        history.apply(
            &mut live,
            messages[550..].to_vec(),
            Some(AppTranscriptCursor {
                revision: "replacement".into(),
                before: 550,
                base: None,
            }),
        );
        assert!(history.context_seek.is_none());
    }

    #[::core::prelude::v1::test]
    fn accepted_remote_seek_event_starts_context_resolution_and_failed_loading_finishes_at_row_start()
     {
        use crate::ui::chat::{Session, scroller::HistorySeek};
        use std::{cell::RefCell, rc::Rc};
        let mut app = ::gpui::TestAppContext::single();
        app.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        let window = app.add_empty_window();
        let (mut history, live) = fixture(&corpus(200..500), 350);
        history.context_seek = None;
        let chat = window.update(|window, cx| {
            cx.new(|cx| {
                let mut session = Session::new("/synthetic/context-event".into());
                session.transcript = live;
                let mut chat = Chat::base(
                    None,
                    WeakEntity::new_invalid(),
                    session,
                    Rc::new(|_, _, _| None),
                    window,
                    cx,
                );
                chat.history = history;
                chat.scroller
                    .update(cx, |scroller, cx| scroller.splice(0..0, 50, cx));
                chat.sync_history_range(cx);
                chat
            })
        });
        let scroller = window.update(|_, cx| chat.read(cx).scroller.clone());
        let events = Rc::new(RefCell::new(Vec::new()));
        let _events = window.update(|_, cx| {
            let events = events.clone();
            cx.subscribe(&scroller, move |_, event: &HistorySeek, _| {
                events.borrow_mut().push(event.0)
            })
        });
        window.update(|_, cx| {
            scroller.update(cx, |scroller, cx| {
                // This is the exact publication made by load_history_seek after
                // accept_seek installs the target page. Chat::base owns the real
                // HistorySeek subscription; there is no manual resolver startup.
                scroller.scroll_to_history(
                    ScrollTarget {
                        row: 350,
                        fraction: 0.5,
                    },
                    cx,
                );
            })
        });
        app.run_until_parked();
        assert!(
            events
                .borrow()
                .iter()
                .any(|target| target.row == 350 && target.fraction == 0.)
        );
        chat.read_with(&app, |chat, _| {
            assert!(chat.history.context_seek.is_none());
            assert!(!chat.history.loading);
        });
    }
}

impl History {
    fn context_step(&self, live: &Transcript, compact: bool) -> Option<ContextStep> {
        let seek = self.context_seek.as_ref()?;
        Some(seek.step(self.displayed(live).entries(), self.range(live)?, compact))
    }

    pub(super) fn context_visible(&self, visible: Range<usize>) -> Range<usize> {
        let Some(target) = self.context_seek.as_ref().map(|seek| seek.target) else {
            return visible;
        };
        let start = self
            .archive
            .as_ref()
            .map_or_else(|| self.live_start(), |window| window.start);
        let Some(row) = target.row.checked_sub(start) else {
            return visible;
        };
        visible.start.min(row)..visible.end.max(row + 1)
    }
}

impl Chat {
    /// Partial groups larger than the context budget remain unmeasured, but
    /// still permit bounded navigation to the requested raw row's start.
    pub(super) fn resume_history_context(&mut self, cx: &mut Context<Self>) {
        let compact = crate::ui::prefs::current(cx).view.compact_tools;
        match self.history.context_step(&self.session.transcript, compact) {
            Some(ContextStep::Load(direction)) => self.load_history(direction, cx),
            Some(ContextStep::RowStart(target)) => {
                self.history.context_seek = None;
                self.scroller
                    .update(cx, |state, cx| state.scroll_to_history(target, cx));
            }
            Some(ContextStep::Complete) => self.history.context_seek = None,
            None => {}
        }
    }

    pub(super) fn stop_history_context(&mut self, cx: &mut Context<Self>) {
        if let Some(seek) = self.history.context_seek.take() {
            self.scroller.update(cx, |state, cx| {
                state.scroll_to_history(
                    ScrollTarget {
                        fraction: 0.,
                        ..seek.target
                    },
                    cx,
                )
            });
        }
    }
}
