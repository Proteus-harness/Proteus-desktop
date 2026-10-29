//! Transient send receipts and the root run's visible activity. These never
//! enter the persisted transcript: only server events create chat entries.
// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (the generating caption follows run state and yields to interactive requests).

use std::collections::HashSet;

use proteus_contracts::app_protocol::{AppExecutionState, AppQueuedUserMessage, AppRunStatus};
use proteus_contracts::domain::{Event, ImageAttachment, ImageRef, MessageId};

#[derive(Default)]
pub(super) struct Feedback {
    sends: Vec<PendingSend>,
    run: Option<String>,
    phase: Phase,
}

pub(super) struct PendingSend {
    pub id: String,
    pub text: String,
    pub images: Vec<String>,
    accepted: bool,
    observed: bool,
    message_id: Option<MessageId>,
    queue_baseline: HashSet<MessageId>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Idle,
    Thinking,
    Writing,
    Tools,
}

impl Feedback {
    pub fn begin(
        &mut self,
        id: String,
        text: String,
        images: &[ImageAttachment],
        queued: &[AppQueuedUserMessage],
    ) {
        self.sends.push(PendingSend {
            id,
            text,
            images: images.iter().map(|image| image.name.clone()).collect(),
            accepted: false,
            observed: false,
            message_id: None,
            queue_baseline: queued.iter().map(|message| message.message_id).collect(),
        });
    }

    pub fn visible_sends(&self) -> impl Iterator<Item = &PendingSend> {
        self.sends.iter().filter(|send| !send.observed)
    }

    /// A delayed connection may have many local sends. The footer shows only
    /// the latest one and a count, while retaining every complete receipt.
    pub fn latest_receipt(&self) -> Option<(&PendingSend, usize)> {
        self.visible_sends()
            .enumerate()
            .last()
            .map(|(before, send)| (send, before))
    }

    /// Successful HTTP acceptance alone does not remove the visible receipt:
    /// SSE can arrive later. A failure after a server echo must not restore an
    /// already delivered draft, even if the HTTP connection failed afterwards.
    pub fn complete(&mut self, id: &str, success: bool, message_id: Option<MessageId>) -> bool {
        let Some(index) = self.sends.iter().position(|send| send.id == id) else {
            return false;
        };
        if success && !self.sends[index].observed {
            self.sends[index].accepted = true;
            self.sends[index].message_id = message_id;
            return false;
        }
        let send = self.sends.remove(index);
        !success && !send.observed
    }

    pub fn submitted(&mut self, text: &str, images: &[ImageRef]) {
        if let Some(send) = self.sends.iter_mut().find(|send| {
            !send.observed
                && send.text == text
                && send.images.len() == images.len()
                && send
                    .images
                    .iter()
                    .zip(images)
                    .all(|(name, image)| *name == image.name)
        }) {
            send.observed = true;
        }
        self.prune();
    }

    pub fn queued(&mut self, messages: &[AppQueuedUserMessage]) {
        for message in messages {
            if let Some(send) = self.sends.iter_mut().find(|send| {
                !send.observed
                    && !send.queue_baseline.contains(&message.message_id)
                    && send.text == message.text
                    && send.images.len() == message.image_count
            }) {
                send.observed = true;
            }
            // The same queued message in a later snapshot cannot consume a
            // second identical local send.
            for send in &mut self.sends {
                send.queue_baseline.insert(message.message_id);
            }
        }
        self.prune();
    }

    /// A reconnect snapshot is an authoritative handoff even when the user
    /// row has already left its bounded live tail. Correlate by the request's
    /// run id, never by an old identical message's text.
    pub fn snapshot(
        &mut self,
        execution: &AppExecutionState,
        tools_running: bool,
        message_ids: &[MessageId],
    ) {
        for send in &mut self.sends {
            if execution
                .active
                .iter()
                .chain(&execution.last)
                .any(|run| run.run_id == send.id)
                || send.message_id.is_some_and(|id| message_ids.contains(&id))
            {
                send.observed = true;
            }
        }
        self.prune();
        self.execution(execution);
        if tools_running && self.run.is_some() {
            self.phase = Phase::Tools;
        }
    }

    pub fn execution(&mut self, execution: &AppExecutionState) {
        let active = execution
            .active
            .as_ref()
            .filter(|run| run.status == AppRunStatus::Running);
        let run = active.map(|run| run.run_id.clone());
        if self.run != run {
            self.phase = if run.is_some() {
                Phase::Thinking
            } else {
                Phase::Idle
            };
            self.run = run;
        }
    }

    /// The caller filters child-thread events. Tool activity and streamed text
    /// have their own visible rows; the thinking label yields to them.
    pub fn runtime(&mut self, event: &Event) -> bool {
        let before = self.phase;
        self.phase = match event {
            Event::TurnStarted { .. }
            | Event::ModelRequestPrepared { .. }
            | Event::AssistantReasoningDelta { .. } => Phase::Thinking,
            Event::AssistantTextDelta { .. } | Event::AssistantMessageCompleted { .. } => {
                Phase::Writing
            }
            Event::ToolCallRequested { .. }
            | Event::ToolCallResolved { .. }
            | Event::AssistantToolArgsDelta { .. } => Phase::Tools,
            Event::ToolFinished { .. } => Phase::Thinking,
            Event::TurnFinished { .. } | Event::Error { .. } => Phase::Idle,
            _ => before,
        };
        before != self.phase
    }

    pub fn thinking(&self, execution: &AppExecutionState, waiting_for_user: bool) -> bool {
        !waiting_for_user
            && execution
                .active
                .as_ref()
                .is_some_and(|run| run.status == AppRunStatus::Running)
            && self.phase == Phase::Thinking
    }

    pub fn clear_activity(&mut self) {
        self.phase = Phase::Idle;
        self.run = None;
    }

    pub fn fail_activity(&mut self) {
        // Keep the run identity so an unchanged execution update cannot
        // resurrect a failed indicator before a new run or reconnect.
        self.phase = Phase::Idle;
    }

    fn prune(&mut self) {
        self.sends.retain(|send| !(send.accepted && send.observed));
    }
}

impl PendingSend {
    pub fn preview(&self) -> String {
        preview(&self.text)
    }

    pub fn status(&self) -> &'static str {
        if self.accepted {
            "Отправлено"
        } else {
            "Отправляю…"
        }
    }
}

/// Bound both height and work before the canonical message arrives. A draft
/// made of hundreds of short lines must not push the composer off screen.
fn preview(text: &str) -> String {
    let mut result = String::new();
    let mut lines = 1;
    for (index, ch) in text.chars().enumerate() {
        if index == 320 || (ch == '\n' && lines == 4) {
            break;
        }
        if ch == '\n' {
            lines += 1;
        }
        result.push(ch);
    }
    result
}

#[cfg(test)]
mod tests;
