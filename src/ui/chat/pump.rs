//! The live session stream: connection state and server events folded into
//! the chat.

use gpui_kit::*;
use proteus_contracts::app_protocol::AppServerEvent;
use proteus_contracts::domain::Event as RuntimeEvent;

use super::context_usage::ContextUsage;
use super::row_layout::{reconciles_rows, row_layout};
use super::stream_batch::{BatchItem, batches, ready_batch, yield_after_batch};
use super::{Chat, ChatEvent};
use crate::connection::PumpItem;
use crate::transcript::Change;

/// Streams the session into the chat until the chat is dropped; dropping
/// the task closes the channel, which stops the stream thread.
pub(super) fn start(events: async_channel::Receiver<PumpItem>, cx: &mut Context<Chat>) -> Task<()> {
    cx.spawn(async move |this, cx| {
        while let Ok(first) = events.recv().await {
            let batch = ready_batch(first, &events);
            let applied = this.update(cx, |this, cx| {
                let shown = this.apply_batch(batch, cx);
                // Copies of other sessions' activity and runtime events
                // without a trace in the chat leave it as drawn.
                if shown {
                    cx.notify();
                }
            });
            if applied.is_err() {
                break;
            }
            yield_after_batch().await;
        }
    })
}

impl Chat {
    /// Partition only the already-ready batch; no timer delays incoming text.
    pub(super) fn apply_batch(&mut self, batch: Vec<PumpItem>, cx: &mut Context<Self>) -> bool {
        let mut shown = false;
        for item in batches(batch) {
            match item {
                BatchItem::Barrier(item) => shown |= self.apply(item, cx),
                BatchItem::Text(run) => {
                    let outcome = self.session.apply_text_run(run);
                    if !self.history.archived() {
                        for change in outcome.layout_changes() {
                            self.apply_change(change, cx);
                        }
                    }
                    if outcome.shown() {
                        self.sync_history_range(cx);
                    }
                    for change in outcome.event_changes() {
                        cx.emit(ChatEvent::SessionEntriesChanged(change));
                    }
                    shown |= outcome.shown();
                }
            }
        }
        shown
    }

    /// Folds one stream item in; true when the chat looks different.
    pub(super) fn apply(&mut self, item: PumpItem, cx: &mut Context<Self>) -> bool {
        let session = &mut self.session;
        match item {
            PumpItem::Connected => {
                // A reconnect baseline does not carry profile/config state.
                // Initial setup already starts its own config request.
                let reconnecting = session.link_error.is_some();
                self.context_requests.invalidate();
                session.cursor.begin_connection();
                session.pending_cursor.begin_connection();
                session.connected = true;
                session.link_error = None;
                if reconnecting {
                    self.load_config(cx);
                }
                cx.emit(ChatEvent::SessionStateChanged);
                true
            }
            PumpItem::Disconnected(error) => {
                session.feedback.clear_activity();
                session.connected = false;
                session.link_error = Some(error.into());
                cx.emit(ChatEvent::SessionStateChanged);
                true
            }
            PumpItem::Event(event) => self.apply_event(*event, cx),
        }
    }

    fn apply_event(&mut self, event: AppServerEvent, cx: &mut Context<Self>) -> bool {
        if matches!(event, AppServerEvent::SessionCatalogUpdated) {
            cx.emit(ChatEvent::CatalogUpdated);
            return false;
        }
        if let AppServerEvent::SessionActivityUpdated {
            session_dir,
            activity,
        } = event
        {
            cx.emit(ChatEvent::Activity {
                session_dir,
                activity,
            });
            return false;
        }
        let session = &mut self.session;
        if !session.cursor.accept(&event) {
            return false;
        }
        let state_changed = match &event {
            AppServerEvent::Runtime { envelope } => {
                matches!(envelope.event, RuntimeEvent::TokenUsageUpdated { .. })
            }
            _ => true,
        };
        let refresh_context = matches!(&event, AppServerEvent::SessionSnapshot { .. });
        let mut shown = true;
        let mut display_changes = None;
        let change = match event {
            AppServerEvent::SessionSnapshot { snapshot } => {
                crate::startup_trace::first("chat.snapshot.applied");
                let settled = session.snapshot_settles_run(&snapshot);
                let cancelled = self.history.request_pending();
                let (change, display) = session.apply_snapshot(*snapshot, &mut self.history);
                if cancelled {
                    self.scroller
                        .update(cx, |state, cx| state.history_ready(cx));
                }
                if settled {
                    cx.emit(ChatEvent::RunSettled);
                }
                display_changes = Some(display);
                change
            }
            AppServerEvent::ExecutionUpdated { execution } => {
                let settled = session.running() && execution.active.is_none();
                session.feedback.execution(&execution);
                session.execution = execution;
                if settled {
                    self.load_context(cx);
                    cx.emit(ChatEvent::RunSettled);
                }
                Change::None
            }
            AppServerEvent::Runtime { envelope } => {
                let feedback_changed = session
                    .root_thread
                    .is_none_or(|root| root == envelope.thread_id)
                    && session.feedback.runtime(&envelope.event);
                if let RuntimeEvent::TokenUsageUpdated { usage, .. } = &envelope.event
                    && session
                        .root_thread
                        .is_none_or(|root| root == envelope.thread_id)
                {
                    self.context_requests.invalidate();
                    session.context = ContextUsage::from_event(usage);
                } else {
                    shown = feedback_changed;
                }
                let change = session
                    .transcript
                    .apply_runtime(envelope.thread_id, &envelope.event);
                session.transcript.stamp(change, envelope.timestamp_ms);
                change
            }
            AppServerEvent::UserMessageSubmitted { text, images } => {
                session.feedback.submitted(&text, &images);
                let change = session.transcript.push_user(None, &text, images);
                let now = chrono::Utc::now().timestamp_millis();
                session.transcript.stamp(change, now);
                change
            }
            AppServerEvent::PendingRequestsUpdated { snapshot } => {
                if session.pending_cursor.accept_stream(
                    &snapshot.session_id.to_string(),
                    &snapshot.stream_id,
                    snapshot.seq,
                ) {
                    let shown = session.pending.as_ref();
                    for key in super::requests::request_keys(&snapshot) {
                        if !shown.is_some_and(|shown| {
                            super::requests::request_keys(shown).any(|old| old == key)
                        }) {
                            self.fresh_requests.mark(key);
                        }
                    }
                    session.feedback.queued(&snapshot.queued_user_messages);
                    session.pending = Some(*snapshot);
                }
                if let Some(pending) = &session.pending {
                    self.user_input.retain(pending);
                }
                Change::None
            }
            AppServerEvent::EventStreamLagged { .. }
            | AppServerEvent::ModulesReloaded { .. }
            | AppServerEvent::ProfileReloadStatus { error: None } => {
                self.load_config(cx);
                Change::None
            }
            AppServerEvent::Error { message } => {
                session.feedback.fail_activity();
                self.show_notice(message, cx);
                Change::None
            }
            AppServerEvent::ProfileReloadStatus { error: Some(error) } => {
                self.show_notice(format!("Не удалось перезагрузить профиль: {error}"), cx);
                Change::None
            }
            AppServerEvent::Shutdown => {
                session.feedback.clear_activity();
                cx.emit(ChatEvent::ServerStopped);
                Change::None
            }
            _ => Change::None,
        };
        shown |= !matches!(change, Change::None);
        if let Some(changes) = display_changes {
            self.apply_history_changes(changes, cx);
        } else if !self.history.archived() {
            self.apply_change(change, cx);
        }
        self.sync_history_range(cx);
        if refresh_context {
            self.load_context(cx);
        }
        if !matches!(change, Change::None) {
            cx.emit(ChatEvent::SessionEntriesChanged(change));
        }
        if state_changed {
            cx.emit(ChatEvent::SessionStateChanged);
        }
        shown
    }

    pub(super) fn apply_change(&mut self, change: Change, cx: &mut Context<Self>) {
        if reconciles_rows(change) {
            let entries = self.history.displayed(&self.session.transcript).entries();
            let keys = entries.iter().map(|entry| entry.key).collect();
            self.replies.retain(&keys);
            self.tool_diffs.retain(|key, _| keys.contains(key));
            self.expanded.retain(|key| keys.contains(key));
            self.full_blocks.retain(|(key, _)| keys.contains(key));
            self.open_chains.retain(|key| keys.contains(key));
            self.sent_images.retain(entries);
        }
        let layout = row_layout(self.display_transcript().entries(), change);
        if let Some(layout) = layout {
            if let Some(key) = layout.fresh_key {
                self.fresh_rows.mark(key);
            }
            self.scroller
                .update(cx, |scroller, cx| match layout.primary {
                    Change::Pushed => scroller.append(1, cx),
                    Change::Updated(row) => scroller.remeasure_items(row..row + 1, cx),
                    _ => unreachable!("row layout contains a push or update"),
                });
            if let Some(rows) = layout.chain {
                self.scroller
                    .update(cx, |scroller, cx| scroller.remeasure_items(rows, cx));
            }
        } else if let Change::Spliced {
            start,
            removed,
            inserted,
        } = change
        {
            self.scroller.update(cx, |scroller, cx| {
                scroller.splice(start..start + removed, inserted, cx)
            });
        }
    }
}
