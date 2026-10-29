//! Server calls of a chat: sending, stopping, answers and session config.

use gpui_kit::*;
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
                this.apply_config(config, cx);
                let generation = this.config_generation;
                let dir = this.session.dir.clone();
                this.call(
                    cx,
                    move |api| api.config(&dir),
                    move |this, config, cx| {
                        if this.config_generation == generation {
                            this.apply_config(config, cx);
                        }
                    },
                );
            },
        );
    }

    pub(super) fn load_context(&mut self, cx: &mut Context<Self>) {
        if let Some(generation) = self.context_requests.request() {
            self.fetch_context(generation, cx);
        }
    }

    fn fetch_context(&mut self, generation: u64, cx: &mut Context<Self>) {
        let dir = self.session.dir.clone();
        let api = self.api.clone();
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
                            cx.emit(ChatEvent::SessionStateChanged);
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
        let draft_text = self.composer.read(cx).value().to_string();
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
            images: draft.images(),
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
        self.session
            .feedback
            .begin(request_id.clone(), text.clone(), &request.images, queued);
        self.composer.update(cx, |state, cx| {
            state.clean(window, cx);
            state.focus(window, cx);
        });
        self.clear_notice(cx);
        self.show_latest(cx);
        cx.emit(ChatEvent::Sent);
        cx.notify();
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.send(&request) })
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
                            .and_then(|value| value.get("message_id"))
                            .and_then(|id| serde_json::from_value(id.clone()).ok());
                        let restore =
                            this.session
                                .feedback
                                .complete(&request_id, result.is_ok(), message_id);
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
                        this.show_notice(format!("Не отправлено: {error:#}"), cx);
                        cx.notify();
                    })
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        let Some(run) = &self.session.execution.active else {
            return;
        };
        let (dir, run_id) = (self.session.dir.clone(), run.run_id.clone());
        self.call(cx, move |api| api.cancel(&dir, &run_id), |_, _, _| {});
    }

    /// Answers an approval; `cache` lets the server repeat a yes without
    /// asking.
    pub(super) fn resolve_approval(
        &mut self,
        approval_id: String,
        approved: bool,
        cache: ApprovalCacheScope,
        cx: &mut Context<Self>,
    ) {
        let dir = self.session.dir.clone();
        self.call(
            cx,
            move |api| api.approve(&dir, &approval_id, approved, cache),
            |_, _, _| {},
        );
    }

    /// Empty-composer Enter follows the visible dock section. A collapsed
    /// request first opens; a second Enter answers its displayed details.
    pub(super) fn confirm_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((activity, open)) = self.open_pending_activity(cx) else {
            return false;
        };
        if !open {
            return true;
        }
        match activity {
            ActivityId::Approval(id) => {
                let live = self.session.pending.as_ref().is_some_and(|pending| {
                    pending
                        .approvals
                        .iter()
                        .any(|request| request.approval_id == id)
                });
                if live {
                    self.resolve_approval(id, true, ApprovalCacheScope::None, cx);
                }
                live
            }
            ActivityId::Question(id) => {
                let live = self.user_input_request(&id).is_some();
                if live {
                    self.next_question(&id, window, cx);
                }
                live
            }
            ActivityId::Plan => false,
        }
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
        self.session.config = Some(config);
        cx.emit(ChatEvent::SessionStateChanged);
    }
}
