// Адаптировано из Zed: crates/agent_ui/src/conversation_view/message_queue.rs
// (failed sends retain their complete content instead of replacing new input).
use gpui_kit::*;

use super::Chat;
use super::attachments::Attachments;

/// A complete unsent message kept apart from whatever is being typed now.
pub(super) struct SavedDraft {
    pub id: String,
    pub text: String,
    pub attachments: Attachments,
}

impl SavedDraft {
    pub fn new(text: String, attachments: Attachments) -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            text,
            attachments,
        }
    }
}

impl Chat {
    pub(super) fn has_draft(&self, cx: &App) -> bool {
        !self.composer.read(cx).value().trim().is_empty()
            || !self.attachments.is_empty()
            || self.attachments_loading
    }

    /// Restoring never overwrites work typed since the failed request. Swap
    /// the current draft into the retained messages before opening this one.
    pub(super) fn restore_draft(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.attachments_loading {
            return;
        }
        let Some(index) = self.saved_drafts.iter().position(|draft| draft.id == id) else {
            return;
        };
        let saved = self.saved_drafts.remove(index);
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        if self.has_draft(cx) {
            self.saved_drafts.push(SavedDraft::new(
                self.composer.read(cx).value().to_string(),
                std::mem::take(&mut self.attachments),
            ));
        }
        self.attachments = saved.attachments;
        self.composer.update(cx, |state, cx| {
            state.set_value(saved.text, window, cx);
            state.focus(window, cx);
        });
        cx.notify();
    }
}
