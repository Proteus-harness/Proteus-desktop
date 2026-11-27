//! Server calls of a chat: sending, stopping, answers and session config.

use gpui_kit::*;
use proteus_contracts::app_protocol::AppRunStatus;
use proteus_contracts::app_protocol::config::ConfigSummary;
use proteus_contracts::app_protocol::http::SendRequest;
use proteus_contracts::contracts::{ApprovalCacheScope, UserInputRequest, UserInputResponse};
use proteus_contracts::domain::{MessageId, RunOptions};

use super::activity_dock::ActivityId;
use super::context_usage::ContextUsage;
use super::draft::SavedDraft;
use super::{Chat, ChatEvent};

impl Chat {
    pub fn load_config(&mut self, cx: &mut Context<Self>) {
        if !self.supports_core() {
            return;
        }
        self.config_generation = self.config_generation.wrapping_add(1);
        let generation = self.config_generation;
        let dir = self.session.dir.clone();
        self.call(
            cx,
            move |api| api.local_config(&dir),
            move |this, config, cx| {
                if this.config_generation != generation {
                    return;
                }
                this.apply_local_config(config, cx);
            },
        );
    }

    pub(in crate::ui) fn apply_local_config(
        &mut self,
        mut config: ConfigSummary,
        cx: &mut Context<Self>,
    ) {
        if let Some(previous) = &self.session.config {
            // Keep discovered choices usable while the next catalog loads.
            // The local response already knows which providers are configured.
            let providers: std::collections::HashSet<_> = config
                .model_options
                .iter()
                .map(|option| option.provider.as_str())
                .collect();
            let cached: Vec<_> = previous
                .model_options
                .iter()
                .filter(|option| providers.contains(option.provider.as_str()))
                .collect();
            let keys: std::collections::HashSet<_> = cached
                .iter()
                .map(|option| (option.provider.as_str(), option.name.as_str()))
                .collect();
            config
                .model_options
                .retain(|option| !keys.contains(&(option.provider.as_str(), option.name.as_str())));
            config.model_options.extend(cached.into_iter().cloned());
        }
        self.apply_config(config, cx);
        let generation = self.config_generation;
        let dir = self.session.dir.clone();
        self.call_result(
            cx,
            move |api| api.config(&dir),
            move |this, result, cx| {
                if this.config_generation != generation {
                    return;
                }
                match result {
                    Ok(config) => this.apply_config(config, cx),
                    Err(error) => this.show_notice(format!("{error:#}"), cx),
                }
            },
        );
    }

    pub(super) fn load_context(&mut self, cx: &mut Context<Self>) {
        if !self.supports_core() {
            return;
        }
        if let Some(generation) = self.context_requests.request() {
            self.fetch_context(generation, cx);
        }
    }

    fn fetch_context(&mut self, generation: u64, cx: &mut Context<Self>) {
        let dir = self.session.dir.clone();
        let Some(api) = self.api.clone() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.context(&dir) })
                .await;
            this.update(cx, |this, cx| {
                let completed = this.context_requests.finish(generation);
                if completed.apply {
                    match result {
                        Ok(context) => {
                            this.session.context = context
                                .latest_usage
                                .as_ref()
                                .and_then(ContextUsage::from_snapshot);
                            cx.emit(ChatEvent::state_changed());
                        }
                        Err(error) => this.show_notice(format!("{error:#}"), cx),
                    }
                    cx.notify();
                }
                if let Some(generation) = completed.next {
                    this.fetch_context(generation, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Sends the composer text and images; the server queues them while a
    /// run is active. A refused message returns to an empty composer.
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.readonly.is_some() {
            return;
        }
        if self.codex.is_some() {
            self.send_codex(window, cx);
            return;
        }
        let draft_source = self.composer.read(cx).text();
        let draft_text = match draft_source.as_str() {
            Some(text) => text.to_owned(),
            None => String::from(draft_source),
        };
        let text = draft_text.trim().to_owned();
        if (text.is_empty() && self.attachments.is_empty())
            || !self.session.connected
            || self.attachments_loading
        {
            return;
        }
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        let draft = std::mem::take(&mut self.attachments);
        let request = SendRequest {
            id: Some(format!("run-{}", uuid::Uuid::new_v4().simple())),
            text: text.clone(),
            images: Vec::new(),
            options: RunOptions::default(),
            session_dir: self.session.dir.clone(),
        };
        let request_id = request.id.clone().expect("local sends have a request id");
        let queued = self
            .session
            .pending
            .as_ref()
            .map(|pending| pending.queued_user_messages.as_slice())
            .unwrap_or_default();
        self.session.feedback.begin_with_names(
            request_id.clone(),
            text.clone(),
            draft.image_names(),
            queued,
        );
        self.composer.update(cx, |state, cx| {
            state.clean(window, cx);
            state.focus(window, cx);
        });
        self.clear_notice(cx);
        self.show_latest(cx);
        cx.emit(ChatEvent::Sent);
        cx.notify();
        let Some(api) = self.api.clone() else {
            return;
        };
        let images = draft.images();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut request = request;
                    request.images = images
                        .into_iter()
                        .map(|image| image.encode())
                        .collect::<anyhow::Result<_>>()
                        .map_err(|error| format!("Не удалось подготовить вложения: {error:#}"))?;
                    let receipt = api.send(&request).map_err(|error| {
                        format!("Core не подтвердил приём сообщения.\n{error:#}")
                    })?;
                    let first_input = receipt.get("accepted").and_then(serde_json::Value::as_bool)
                        == Some(true)
                        && api.finish_session_draft(&request.session_dir);
                    Ok::<_, String>((receipt, first_input))
                })
                .await;
            let Ok(owner) = this.read_with(cx, |this, _| this.window_handle) else {
                return;
            };
            owner
                .update(cx, |_, window, cx| {
                    this.update(cx, |this, cx| {
                        let message_id = result
                            .as_ref()
                            .ok()
                            .and_then(|(receipt, _)| receipt.get("message_id"))
                            .and_then(|id| serde_json::from_value(id.clone()).ok());
                        let restore =
                            this.session
                                .feedback
                                .complete(&request_id, result.is_ok(), message_id);
                        if result.as_ref().is_ok_and(|(_, first_input)| *first_input) {
                            cx.emit(ChatEvent::CatalogUpdated);
                        }
                        let Err(error) = result else {
                            cx.notify();
                            return;
                        };
                        if !restore {
                            // The server already echoed this message. Restoring it
                            // after a transport error would encourage a duplicate.
                            cx.notify();
                            return;
                        }
                        if !this.has_draft(cx) {
                            this.composer
                                .update(cx, |state, cx| state.set_value(draft_text, window, cx));
                            this.attachments = draft;
                        } else {
                            this.saved_drafts.push(SavedDraft::new(draft_text, draft));
                        }
                        this.show_error(error, cx);
                        cx.notify();
                    })
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn stop_from_keyboard(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.session.connected || !self.session.running() {
            return false;
        }
        self.stop(cx);
        true
    }

    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        if self.codex.is_some() {
            self.stop_codex(cx);
            return;
        }
        let Some(run) = &self.session.execution.active else {
            return;
        };
        let (dir, run_id) = (self.session.dir.clone(), run.run_id.clone());
        let expected = run_id.clone();
        self.call_result(
            cx,
            move |api| api.cancel(&dir, &run_id),
            move |this, result, cx| {
                if let Err(error) = result
                    && this.session.execution.active.as_ref().is_some_and(|run| {
                        run.run_id == expected && run.status == AppRunStatus::Running
                    })
                {
                    this.show_error(
                        format!("Core не подтвердил запрос на остановку.\n{error:#}"),
                        cx,
                    );
                }
            },
        );
    }

    /// Resolve only the displayed live requests. Retain acknowledged IDs until
    /// the SSE snapshot removes them so key repeats and clicks cannot resend.
    pub(super) fn resolve_approval(
        &mut self,
        approval_id: String,
        approved: bool,
        cache: ApprovalCacheScope,
        cx: &mut Context<Self>,
    ) {
        self.resolve_approvals(&[approval_id], approved, cache, cx);
    }

    pub(super) fn resolve_approvals(
        &mut self,
        ids: &[String],
        approved: bool,
        cache: ApprovalCacheScope,
        cx: &mut Context<Self>,
    ) {
        if !self.session.connected || !self.supports_core() {
            return;
        }
        let Some(pending) = self.session.pending.as_ref() else {
            return;
        };
        let live: Vec<_> = ids
            .iter()
            .filter(|id| {
                pending
                    .approvals
                    .iter()
                    .any(|request| &request.approval_id == *id)
                    && self.approvals_in_flight.insert((*id).clone())
            })
            .cloned()
            .collect();
        if live.is_empty() {
            return;
        }
        let dir = self.session.dir.clone();
        let connection = self.core_connection();
        self.call_result(
            cx,
            move |api| {
                let errors: Vec<_> = live
                    .into_iter()
                    .filter_map(|id| {
                        api.approve(&dir, &id, approved, cache)
                            .err()
                            .map(|error| (id, format!("{error:#}")))
                    })
                    .collect();
                Ok(errors)
            },
            move |chat, result, cx| {
                if chat.core_connection() != connection {
                    return;
                }
                if let Ok(errors) = result
                    && !errors.is_empty()
                {
                    for (id, _) in &errors {
                        chat.approvals_in_flight.remove(id);
                    }
                    let details = errors
                        .into_iter()
                        .map(|(_, error)| error)
                        .collect::<Vec<_>>()
                        .join(
                            "
",
                        );
                    chat.show_error(
                        format!(
                            "Не удалось передать решение агенту.
{details}"
                        ),
                        cx,
                    );
                }
            },
        );
    }

    pub(super) fn advance_pending_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ActivityId::Question(id)) = self.activity_dock.pending_target()
            && self.user_input_request(&id).is_some()
            && self.reveal_pending_activity(&ActivityId::Question(id.clone()), cx)
        {
            self.next_question(&id, window, cx);
        }
    }

    /// An explicit shortcut answers only the approval whose details are visible.
    pub(super) fn approve_from_composer(&mut self, held: bool, cx: &mut Context<Self>) -> bool {
        let Some(ActivityId::Approval(id)) = self.activity_dock.pending_target() else {
            return false;
        };
        let activity = ActivityId::Approval(id.clone());
        if !held
            && self.session.connected
            && self.session.pending.as_ref().is_some_and(|pending| {
                pending
                    .approvals
                    .iter()
                    .any(|request| request.approval_id == id)
            })
            && !super::requests::request_fresh(&activity, self)
            && self.reveal_pending_activity(&activity, cx)
        {
            self.resolve_approval(id, true, ApprovalCacheScope::None, cx);
        }
        true
    }

    /// Composer Escape denies only the approval whose details are visible.
    pub(super) fn deny_pending(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(ActivityId::Approval(id)) = self.activity_dock.selected().cloned() else {
            return false;
        };
        let live = self.session.pending.as_ref().is_some_and(|pending| {
            pending
                .approvals
                .iter()
                .any(|request| request.approval_id == id)
        });
        if !live {
            return false;
        }
        self.resolve_approval(id, false, ApprovalCacheScope::None, cx);
        true
    }

    pub(super) fn delete_queued(&mut self, message_id: MessageId, cx: &mut Context<Self>) {
        let dir = self.session.dir.clone();
        self.call(
            cx,
            move |api| api.delete_queued(&dir, message_id),
            |_, _, _| {},
        );
    }

    /// Sends the answers given so far; `dismiss` sends none at all.
    pub(super) fn submit_user_input(
        &mut self,
        request_id: &str,
        dismiss: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.user_input_request(request_id) else {
            return;
        };
        let response = if dismiss {
            UserInputResponse::empty()
        } else {
            self.user_input.response(request, cx)
        };
        let (dir, request_id) = (self.session.dir.clone(), request_id.to_owned());
        self.clear_notice(cx);
        self.call(
            cx,
            move |api| api.answer_user_input(&dir, &request_id, response),
            |_, _, _| {},
        );
    }

    pub(super) fn user_input_request(&self, request_id: &str) -> Option<&UserInputRequest> {
        self.session
            .pending
            .as_ref()?
            .user_inputs
            .iter()
            .find(|request| request.request_id == request_id)
    }

    pub(in crate::ui) fn apply_config(&mut self, config: ConfigSummary, cx: &mut Context<Self>) {
        self.config_generation = self.config_generation.wrapping_add(1);
        if let Some(error) = &config.model_catalog_error {
            self.show_notice(format!("Не удалось загрузить каталог моделей: {error}"), cx);
        }
        if let Some(path) = config.config_path.as_ref().map(std::path::PathBuf::from) {
            let handle = self.window_handle;
            cx.defer(move |cx| {
                handle
                    .update(cx, |_, window, cx| {
                        crate::ui::prefs::set_visual_agent(&path, window, cx)
                    })
                    .ok();
            });
        }
        self.session.config = Some(std::sync::Arc::new(config));
        cx.emit(ChatEvent::state_changed());
    }
}
