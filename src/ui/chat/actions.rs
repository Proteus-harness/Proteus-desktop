//! Server calls of a chat: sending, stopping, answers and session config.

use gpui_kit::*;
use proteus_contracts::app_protocol::config::ConfigSummary;
use proteus_contracts::app_protocol::http::SendRequest;
use proteus_contracts::contracts::{ApprovalCacheScope, UserInputRequest, UserInputResponse};
use proteus_contracts::domain::{MessageId, RunOptions};

use super::context_usage::ContextUsage;
use super::{Chat, ChatEvent};

impl Chat {
    pub fn load_config(&mut self, cx: &mut Context<Self>) {
        let dir = self.session.dir.clone();
        self.call(
            cx,
            move |api| api.config(&dir),
            |this, config, cx| this.apply_config(config, cx),
        );
    }

    pub(super) fn load_context(&mut self, cx: &mut Context<Self>) {
        let dir = self.session.dir.clone();
        self.call(
            cx,
            move |api| api.context(&dir),
            |this, context, cx| {
                this.session.context = context
                    .latest_usage
                    .as_ref()
                    .and_then(ContextUsage::from_snapshot);
                cx.emit(ChatEvent::SessionStateChanged);
            },
        );
    }

    /// Sends the composer text and images; the server queues them while a
    /// run is active. A refused message returns to an empty composer.
    pub(super) fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_owned();
        if (text.is_empty() && self.attachments.is_empty()) || !self.session.connected {
            return;
        }
        let draft = std::mem::take(&mut self.attachments);
        let request = SendRequest {
            id: Some(format!("run-{}", uuid::Uuid::new_v4().simple())),
            text: text.clone(),
            images: draft.images(),
            options: RunOptions::default(),
            session_dir: self.session.dir.clone(),
        };
        self.composer
            .update(cx, |state, cx| state.clean(window, cx));
        self.clear_notice(cx);
        self.scroller
            .update(cx, |scroller, cx| scroller.scroll_to_end(cx));
        cx.emit(ChatEvent::Sent);
        let api = self.api.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.send(&request) })
                .await;
            let Err(error) = result else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if this.composer.read(cx).value().is_empty() {
                    this.composer
                        .update(cx, |state, cx| state.set_value(text, window, cx));
                }
                if this.attachments.is_empty() {
                    this.attachments = draft;
                }
                this.show_notice(format!("Не отправлено: {error:#}"), cx);
                cx.notify();
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

    /// Enter in an empty composer answers what the agent waits on: allows
    /// the first approval once or moves on from the open question.
    pub(super) fn confirm_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(pending) = self.session.pending.as_ref() else {
            return false;
        };
        if let Some(approval) = pending.approvals.first() {
            let id = approval.approval_id.clone();
            self.resolve_approval(id, true, ApprovalCacheScope::None, cx);
            return true;
        }
        let Some(request) = pending.user_inputs.first() else {
            return false;
        };
        let request_id = request.request_id.clone();
        self.next_question(&request_id, window, cx);
        true
    }

    /// Esc denies the first approval.
    pub(super) fn deny_pending(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(approval) = self
            .session
            .pending
            .as_ref()
            .and_then(|pending| pending.approvals.first())
        else {
            return false;
        };
        let id = approval.approval_id.clone();
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
        if let Some(error) = &config.model_catalog_error {
            self.show_notice(format!("Не удалось загрузить каталог моделей: {error}"), cx);
        }
        self.session.config = Some(config);
        cx.emit(ChatEvent::SessionStateChanged);
    }
}
