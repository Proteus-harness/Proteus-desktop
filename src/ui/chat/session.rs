use std::path::PathBuf;

use gpui_kit::SharedString;
use proteus_client_common::pending::PendingCursor;
use proteus_client_common::sync::SessionCursor;
use proteus_contracts::app_protocol::config::ConfigSummary;
use proteus_contracts::app_protocol::{
    AppExecutionState, AppPendingRequests, AppRunStatus, AppServerEvent, AppSessionSnapshot,
};
use proteus_contracts::domain::ThreadId;

use super::context_usage::ContextUsage;
use super::history::History;
use crate::transcript::{Change, TextRun, TextRunOutcome, Transcript};

/// What the client knows about one open session.
pub(in crate::ui) struct Session {
    pub dir: PathBuf,
    pub(super) cursor: SessionCursor,
    pub(super) pending_cursor: PendingCursor,
    pub transcript: Transcript,
    pub execution: AppExecutionState,
    pub pending: Option<AppPendingRequests>,
    pub connected: bool,
    pub link_error: Option<SharedString>,
    pub config: Option<ConfigSummary>,
    pub context: Option<ContextUsage>,
    pub(super) root_thread: Option<ThreadId>,
    pub(super) feedback: super::feedback::Feedback,
}

/// What a chat tab says about the run and the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum ChatState {
    Idle,
    Busy(&'static str),
    Trouble(&'static str),
}

impl Session {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            cursor: SessionCursor::default(),
            pending_cursor: PendingCursor::default(),
            transcript: Transcript::default(),
            execution: AppExecutionState::default(),
            pending: None,
            connected: false,
            link_error: None,
            config: None,
            context: None,
            root_thread: None,
            feedback: super::feedback::Feedback::default(),
        }
    }

    pub fn running(&self) -> bool {
        self.execution.active.is_some()
    }

    /// Keep snapshot routing, transient feedback and bounded history atomic.
    pub(super) fn apply_snapshot(
        &mut self,
        snapshot: AppSessionSnapshot,
        history: &mut History,
    ) -> (Change, Vec<Change>) {
        let tools_running = snapshot.transcript.iter().any(|entry| {
            entry
                .tool
                .as_ref()
                .is_some_and(|tool| tool.status == "running")
        });
        let message_ids: Vec<_> = snapshot
            .transcript
            .iter()
            .filter_map(|entry| entry.message_id)
            .collect();
        self.feedback
            .snapshot(&snapshot.execution, tools_running, &message_ids);
        self.execution = snapshot.execution;
        self.root_thread = snapshot.root_thread_id;
        self.transcript.set_root_thread(snapshot.root_thread_id);
        history.apply(&mut self.transcript, snapshot.transcript, snapshot.history)
    }

    /// Every original event retains its cursor gate and feedback transition.
    pub(super) fn apply_text_run(&mut self, mut run: TextRun) -> TextRunOutcome {
        let mut feedback_changed = false;
        run.retain(|event| {
            if !self.cursor.accept(event) {
                return false;
            }
            if let AppServerEvent::Runtime { envelope } = event
                && self
                    .root_thread
                    .is_none_or(|root| root == envelope.thread_id)
            {
                feedback_changed |= self.feedback.runtime(&envelope.event);
            }
            true
        });
        let mut outcome = self.transcript.apply_text_run(&run);
        outcome.feedback_changed = feedback_changed;
        outcome
    }

    pub(super) fn thinking(&self) -> bool {
        let waiting = !self.connected
            || self.pending.as_ref().is_some_and(|pending| {
                !pending.approvals.is_empty() || !pending.user_inputs.is_empty()
            });
        let tools_running = self.transcript.entries().iter().any(|entry| {
            entry
                .tool
                .as_ref()
                .is_some_and(|tool| tool.status == "running")
        });
        !tools_running && self.feedback.thinking(&self.execution, waiting)
    }

    pub fn state(&self) -> ChatState {
        if !self.connected {
            return match self.link_error {
                Some(_) => ChatState::Trouble("Переподключение…"),
                None => ChatState::Busy("Подключение…"),
            };
        }
        match self.execution.active.as_ref().map(|run| &run.status) {
            Some(AppRunStatus::CancelRequested) => ChatState::Busy("Остановка…"),
            Some(_) => ChatState::Busy("Работает…"),
            None => ChatState::Idle,
        }
    }
}
