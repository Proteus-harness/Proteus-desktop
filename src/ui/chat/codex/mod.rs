//! Codex binds to the common transcript, renderer, input and scroller directly.
mod controls;
mod history;
mod requests;
mod state;
pub(super) use state::RunState;

use super::{Chat, ChatEvent, ComposerExtras, Session};
use crate::agents::codex::{Event, Item, ItemKind, Model, Opened, ThreadHandle};
use crate::transcript::{Change, Transcript};
use gpui_kit::*;
use serde_json::Value;
use std::collections::HashMap;

pub(super) struct Binding {
    pub handle: ThreadHandle,
    pub models: std::sync::Arc<[Model]>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub pending: Vec<requests::Request>,
    pub answers: HashMap<(String, String), Entity<gpui_kit::component::input::InputState>>,
    pub selected: HashMap<(String, String), String>,
    pub responding: Vec<Value>,
    pub archive: Option<Transcript>,
    pub next_cursor: Option<String>,
    pub cursor_chain: Vec<Option<String>>,
    pub page_rows: std::collections::VecDeque<usize>,
    pub oldest_page: usize,
    pub newest_page: usize,
    pub history_loading: bool,
    pub history_exhausted: bool,
    pub history_epoch: u64,
    pub history_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub send_generation: u64,
    pub sending: Option<Send>,
    pub plan: Option<Value>,
}
pub(super) struct Send {
    generation: u64,
    local: String,
    text: String,
    echoed: bool,
}

impl Drop for Binding {
    fn drop(&mut self) {
        self.history_cancel
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

impl Binding {
    fn prune_requests(&mut self) {
        let live: std::collections::HashSet<_> = self
            .pending
            .iter()
            .map(|request| request.id.to_string())
            .collect();
        self.answers
            .retain(|(request, _), _| live.contains(request));
        self.selected
            .retain(|(request, _), _| live.contains(request));
        self.responding.retain(|id| live.contains(&id.to_string()));
    }
}

impl Chat {
    pub(in crate::ui) fn new_codex(
        opened: Opened,
        models: Vec<Model>,
        is_new: bool,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        extras: ComposerExtras,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Opened {
            handle,
            events,
            metadata,
            history,
        } = opened;
        let mut session = Session::new(Default::default());
        session.key = proteus_tabs::ChatKey::Codex(metadata.id.clone());
        session.connected = handle.is_connected();
        let state = RunState::resumed(handle.current_turn());
        session.external = Some(state);
        for item in history.data {
            session.transcript.external_upsert(item);
        }
        session.transcript.external_trim_budget(50, 8 * 1024 * 1024);
        let mut chat = Self::base(None, workspace, session, extras, window, cx);
        chat.composer.update(cx, |state, cx| {
            state.set_placeholder("Сообщение для Codex", window, cx)
        });
        let saved =
            crate::ui::prefs::current(cx)
                .codex_selection
                .selection(&metadata.id, is_new, &models);
        // A catalog default is only a display hint. Send None until the
        // server chose a model or the user explicitly selected one.
        let (model, effort) = match saved {
            Some(saved) => (Some(saved.model), saved.effort),
            None => (metadata.model, metadata.effort),
        };
        chat.codex = Some(Binding {
            handle,
            models: models.into(),
            model,
            effort,
            pending: vec![],
            answers: HashMap::new(),
            selected: HashMap::new(),
            responding: vec![],
            archive: None,
            next_cursor: history.next_cursor,
            cursor_chain: vec![None],
            page_rows: Default::default(),
            oldest_page: 0,
            newest_page: 0,
            history_loading: false,
            history_exhausted: false,
            history_epoch: 0,
            history_cancel: Default::default(),
            send_generation: 0,
            sending: None,
            plan: None,
        });
        let rows = chat.session.transcript.entries().len();
        chat.apply_change(
            Change::Spliced {
                start: 0,
                removed: 0,
                inserted: rows,
            },
            cx,
        );
        chat._pump = Some(cx.spawn(async move |this, cx| {
            while let Ok(first) = events.recv().await {
                let mut batch = vec![first];
                while batch.len() < 128 {
                    match events.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(_) => break,
                    }
                }
                if this
                    .update(cx, |this, cx| {
                        for event in batch {
                            this.apply_codex(event, cx);
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
                super::stream_batch::yield_after_batch().await;
            }
            let reason = events
                .disconnect_reason()
                .unwrap_or_else(|| "Codex app-server закрыл соединение".into());
            this.update(cx, |this, cx| {
                this.apply_codex(Event::Disconnected(reason), cx)
            })
            .ok();
        }));
        chat
    }

    fn apply_codex(&mut self, event: Event, cx: &mut Context<Self>) {
        if let Event::Resynced {
            current_turn,
            history,
        } = event
        {
            // Resubscription repairs the existing chat. Keep its draft, frozen
            // reader window and scroll anchor; only reconcile the bounded tail.
            for item in history {
                self.apply_codex(Event::ItemUpsert(item), cx);
            }
            let binding = self.codex.as_ref().expect("Codex binding");
            let state = self.session.external.as_mut().expect("Codex state");
            *state = RunState::resumed(current_turn);
            state.waiting = !binding.pending.is_empty();
            self.session.connected = binding.handle.is_connected();
            self.session.link_error = None;
            cx.emit(ChatEvent::SessionStateChanged);
            return;
        }
        self.pause_codex_before_eviction(&event, cx);
        let Some(binding) = &mut self.codex else {
            return;
        };
        let state = self.session.external.as_mut().expect("Codex state");
        let mut settled = false;
        let mut rejected = None;
        let change = match event {
            Event::SubscriptionSuspended => {
                self.session.connected = false;
                Change::None
            }
            Event::ResyncStarted => {
                self.session.connected = false;
                binding.pending.clear();
                binding.answers.clear();
                binding.selected.clear();
                binding.responding.clear();
                state.waiting = false;
                Change::None
            }
            Event::Resynced { .. } => unreachable!(),
            Event::ItemUpsert(item) => {
                if let ItemKind::User { text } = &item.kind {
                    if let Some(send) = &mut binding.sending {
                        if !send.echoed && send.text.trim() == text.trim() {
                            self.session.transcript.external_reidentify(
                                "",
                                &send.local,
                                &item.turn_id,
                                &item.id,
                            );
                            send.echoed = true;
                        }
                    }
                }
                state.output = matches!(
                    &item.kind,
                    ItemKind::Assistant { .. } | ItemKind::Tool { .. }
                );
                self.session.transcript.external_upsert(item)
            }
            Event::TextDelta { id, turn_id, delta } => {
                state.output = true;
                self.session
                    .transcript
                    .external_delta(&turn_id, &id, &delta, false, false)
            }
            Event::ReasoningDelta { id, turn_id, delta } => {
                state.output = false;
                self.session
                    .transcript
                    .external_delta(&turn_id, &id, &delta, true, false)
            }
            Event::ToolOutputDelta { id, turn_id, delta } => {
                state.output = true;
                self.session
                    .transcript
                    .external_delta(&turn_id, &id, &delta, false, true)
            }
            Event::TurnStarted { id } => {
                state.started(id);
                Change::None
            }
            Event::TurnCompleted { id, status, error } => {
                settled = state.completed(&id);
                if settled {
                    binding.pending.retain(|request| {
                        request.params.get("turnId").and_then(Value::as_str) != Some(&id)
                    });
                    binding.prune_requests();
                    if binding.sending.as_ref().is_some_and(|send| send.echoed) {
                        binding.sending = None;
                    }
                    if let Some(error) = error {
                        self.show_notice(format!("Codex: {error}"), cx);
                    } else if status == "interrupted" {
                        self.show_notice("Codex: выполнение остановлено", cx);
                    }
                }
                Change::None
            }
            Event::PendingRequest { id, method, params } => {
                if !binding.pending.iter().any(|request| request.id == id) {
                    let request = requests::Request::projected(id, method, params);
                    if binding.pending.len() >= 32
                        || binding
                            .pending
                            .iter()
                            .map(requests::Request::retained_bytes)
                            .sum::<usize>()
                            .saturating_add(request.retained_bytes())
                            > 8 * 1024 * 1024
                    {
                        rejected = Some(request.id);
                    } else {
                        binding.pending.push(request);
                    }
                }
                state.waiting = !binding.pending.is_empty();
                Change::None
            }
            Event::RequestResolved { id } => {
                binding.pending.retain(|request| request.id != id);
                binding.prune_requests();
                state.waiting = !binding.pending.is_empty();
                Change::None
            }
            Event::Plan(plan) => {
                binding.plan = Some(plan);
                Change::None
            }
            Event::Usage(usage) => {
                self.session.context = usage
                    .pointer("/tokenUsage/modelContextWindow")
                    .and_then(Value::as_u64)
                    .filter(|max| *max > 0)
                    .map(|max| super::context_usage::ContextUsage {
                        used: usage
                            .pointer("/tokenUsage/last/inputTokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0)
                            .min(u32::MAX as u64) as u32,
                        max: max.min(u32::MAX as u64) as u32,
                        trigger: None,
                    });
                Change::None
            }
            Event::Disconnected(error) => {
                state.disconnected();
                binding
                    .history_cancel
                    .store(true, std::sync::atomic::Ordering::Release);
                binding.history_loading = false;
                binding.history_epoch = binding.history_epoch.wrapping_add(1);
                binding.pending.clear();
                binding.answers.clear();
                binding.selected.clear();
                binding.responding.clear();
                self.session.connected = false;
                self.session.link_error = Some(error.clone().into());
                self.show_notice(
                    format!(
                        "Codex отключён: {error}. Отправка отключена; чат и черновик сохранены."
                    ),
                    cx,
                );
                Change::None
            }
        };
        if let Some(id) = rejected {
            self.reject_codex_overflow(id, cx);
        }
        if !matches!(change, Change::None) {
            if self
                .codex
                .as_ref()
                .is_some_and(|binding| binding.archive.is_none())
            {
                self.apply_change(change, cx);
            }
            let trimmed = self
                .session
                .transcript
                .external_trim_budget(50, 8 * 1024 * 1024);
            if self
                .codex
                .as_ref()
                .is_some_and(|binding| binding.archive.is_none())
            {
                self.apply_change(trimmed, cx);
            }
            cx.emit(ChatEvent::SessionEntriesChanged(change));
        }
        cx.emit(ChatEvent::SessionStateChanged);
        if settled {
            cx.emit(ChatEvent::RunSettled);
        }
    }

    pub(super) fn send_codex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.composer.read(cx).value().to_string();
        let text = draft.trim().to_owned();
        if text.is_empty() || !self.session.connected {
            return;
        }
        if !self.attachments.is_empty() {
            self.show_notice(
                "Codex: пока поддерживается только текст; уберите изображения",
                cx,
            );
            return;
        }
        let state = self.session.external.as_mut().expect("Codex state");
        if !state.begin() {
            self.show_notice("Дождитесь завершения Codex или остановите текущий ход", cx);
            return;
        }
        let binding = self.codex.as_mut().expect("Codex binding");
        binding.send_generation += 1;
        let generation = binding.send_generation;
        let local = format!("local-send:{generation}");
        binding.sending = Some(Send {
            generation,
            local: local.clone(),
            text: draft.clone(),
            echoed: false,
        });
        let (handle, model, effort) = (
            binding.handle.clone(),
            binding.model.clone(),
            binding.effort.clone(),
        );
        self.codex_latest(cx);
        let change = self.session.transcript.external_upsert(Item {
            id: local,
            turn_id: String::new(),
            kind: ItemKind::User { text: text.clone() },
            status: "completed".into(),
            timestamp_ms: Some(chrono::Utc::now().timestamp_millis()),
        });
        self.apply_change(change, cx);
        self.composer.update(cx, |state, cx| {
            state.clean(window, cx);
            state.focus(window, cx);
        });
        self.clear_notice(cx);
        cx.emit(ChatEvent::Sent);
        cx.emit(ChatEvent::SessionStateChanged);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { handle.send(&text, model.as_deref(), effort.as_deref()) }).await;
            let Ok(owner) = this.read_with(cx, |this, _| this.window_handle) else { return; };
            owner.update(cx, |_, window, cx| { this.update(cx, |this, cx| {
                let Some(binding) = &mut this.codex else { return; };
                let Some(send) = binding.sending.as_ref().filter(|send| send.generation == generation) else { return; };
                match result {
                    Ok(turn) => this.session.external.as_mut().unwrap().started(turn),
                    Err(error) => {
                        let echoed = send.echoed;
                        let local = send.local.clone();
                        this.session.external.as_mut().unwrap().failed_send();
                        if !echoed {
                            let change = this.session.transcript.external_remove("", &local);
                            if binding.archive.is_none() { this.apply_change(change, cx); }
                            if this.has_draft(cx) { this.saved_drafts.push(super::draft::SavedDraft::new(draft, Default::default())); }
                            else { this.composer.update(cx, |state, cx| state.set_value(draft, window, cx)); }
                        }
                        this.show_notice(format!("Codex: отправка не подтверждена: {error}. Автоматического повтора нет."), cx);
                    }
                }
                cx.emit(ChatEvent::SessionStateChanged); cx.notify();
            }).ok(); }).ok();
        }).detach();
    }

    fn remember_codex_selection(&self, cx: &mut Context<Self>) {
        let binding = self.codex.as_ref().unwrap();
        let Some(model) = &binding.model else {
            return;
        };
        let selection = crate::conversations::CodexSelection {
            model: model.clone(),
            effort: binding.effort.clone(),
        };
        let thread = binding.handle.thread_id().to_owned();
        crate::ui::prefs::update(cx, |prefs| {
            prefs.codex_selection.remember(&thread, selection)
        });
    }

    pub(super) fn stop_codex(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = &self.codex else {
            return;
        };
        let Some(turn) = self
            .session
            .external
            .as_ref()
            .and_then(|state| state.turn.clone())
            .or_else(|| {
                binding
                    .handle
                    .current_turn()
                    .filter(|turn| !turn.is_empty())
            })
        else {
            return;
        };
        let handle = binding.handle.clone();
        self.session.external.as_mut().unwrap().canceling = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { handle.interrupt(&turn) })
                .await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.session.external.as_mut().unwrap().canceling = false;
                    this.show_notice(format!("Не удалось остановить Codex: {error}"), cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
