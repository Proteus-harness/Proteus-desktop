use std::path::PathBuf;

use gpui_kit::SharedString;
use proteus_client_common::pending::PendingCursor;
use proteus_client_common::sync::SessionCursor;
use proteus_contracts::app_protocol::config::ConfigSummary;
use proteus_contracts::app_protocol::{
    AppExecutionState, AppPendingRequests, AppRunStatus, AppServerEvent, AppSessionSnapshot,
};
use proteus_contracts::domain::{Event, EventEnvelope, SessionId, ThreadId};

use super::context_usage::ContextUsage;
use super::history::History;
use crate::transcript::{Change, TextRun, TextRunOutcome, Transcript};

/// What the client knows about one open session.
pub(in crate::ui) struct Session {
    pub dir: PathBuf,
    pub key: proteus_tabs::ChatKey,
    pub(super) external: Option<super::codex::RunState>,
    pub(super) cursor: SessionCursor,
    pub(super) pending_cursor: PendingCursor,
    pub transcript: Transcript,
    pub execution: AppExecutionState,
    pub pending: Option<AppPendingRequests>,
    pub connected: bool,
    pub link_error: Option<SharedString>,
    /// This session's server closed its stream; the shared Core may still live.
    stream_closed: bool,
    pub config: Option<ConfigSummary>,
    pub context: Option<ContextUsage>,
    pub(super) root_thread: Option<ThreadId>,
    snapshot_identity: Option<(SessionId, String)>,
    pub(super) feedback: super::feedback::Feedback,
}

/// What a chat tab says about the run and the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum ChatState {
    Idle,
    Busy(&'static str),
    Trouble(&'static str),
}

/// One persistent root-turn caption. Tool names remain the same identifiers
/// as the transcript; rendering does not parse or clone their arguments.
pub(super) struct Activity<'a> {
    pub text: &'a str,
    pub animated: bool,
}

impl Session {
    /// Durable session identity, independent of stream/reconnect generations.
    pub(super) fn durable_id(&self) -> Option<SessionId> {
        self.snapshot_identity.as_ref().map(|(id, _)| *id)
    }

    pub(super) fn new(dir: PathBuf) -> Self {
        Self {
            key: proteus_tabs::ChatKey::Proteus(dir.clone()),
            dir,
            external: None,
            cursor: SessionCursor::default(),
            pending_cursor: PendingCursor::default(),
            transcript: Transcript::default(),
            execution: AppExecutionState::default(),
            pending: None,
            connected: false,
            link_error: None,
            stream_closed: false,
            config: None,
            context: None,
            root_thread: None,
            snapshot_identity: None,
            feedback: super::feedback::Feedback::default(),
        }
    }

    pub fn running(&self) -> bool {
        self.external
            .as_ref()
            .map_or_else(|| self.execution.active.is_some(), |state| state.running())
    }

    pub(super) fn stream_connected(&mut self) {
        self.cursor.begin_connection();
        self.pending_cursor.begin_connection();
        self.connected = true;
        self.link_error = None;
        self.stream_closed = false;
    }

    pub(super) fn stream_disconnected(&mut self, error: String) {
        self.feedback.clear_activity();
        self.connected = false;
        self.link_error = Some(error.into());
        self.stream_closed = false;
    }

    /// Shutdown belongs to one session stream, including deletion of an open
    /// session. It is not evidence that the shared backend process has died.
    pub(super) fn stream_shutdown(&mut self) {
        self.feedback.clear_activity();
        self.connected = false;
        self.link_error = None;
        self.stream_closed = true;
        self.pending = None;
    }

    /// A baseline can confirm settlement of a run this chat actually observed.
    /// Keep generation identity across reconnect, but never infer a completion
    /// from another server generation or an unrelated historical last run.
    pub(super) fn snapshot_settles_run(&self, snapshot: &AppSessionSnapshot) -> bool {
        self.snapshot_identity
            .as_ref()
            .is_some_and(|(session, stream)| {
                *session == snapshot.session_id && stream == &snapshot.stream_id
            })
            && snapshot.execution.active.is_none()
            && self.execution.active.as_ref().is_some_and(|active| {
                snapshot.execution.last.as_ref().is_some_and(|last| {
                    last.run_id == active.run_id
                        && matches!(
                            last.status,
                            AppRunStatus::Success
                                | AppRunStatus::Error
                                | AppRunStatus::Canceled
                                | AppRunStatus::Timeout
                        )
                })
            })
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
                .is_some_and(|tool| super::tool_row::is_running(&tool.status))
        });
        let message_ids: Vec<_> = snapshot
            .transcript
            .iter()
            .filter_map(|entry| entry.message_id)
            .collect();
        self.feedback
            .snapshot(&snapshot.execution, tools_running, &message_ids);
        self.snapshot_identity = Some((snapshot.session_id, snapshot.stream_id));
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
            if let AppServerEvent::Runtime { envelope } = event {
                feedback_changed |= self.runtime_feedback(envelope);
            }
            true
        });
        let mut outcome = self.transcript.apply_text_run(&run);
        outcome.feedback_changed = feedback_changed;
        outcome
    }

    /// An empty initial snapshot has no root yet. Learn it from the first
    /// TurnStarted before accepting activity; forwarded child tool/error events
    /// must not change the parent's indicator during its first turn.
    pub(super) fn runtime_feedback(&mut self, envelope: &EventEnvelope) -> bool {
        if self.root_thread.is_none() && matches!(envelope.event, Event::TurnStarted { .. }) {
            self.root_thread = Some(envelope.thread_id);
        }
        self.root_thread
            .is_none_or(|root| root == envelope.thread_id)
            && self.feedback.runtime(&envelope.event)
    }

    #[cfg(test)]
    pub(super) fn thinking(&self) -> bool {
        if let Some(external) = &self.external {
            return self.connected && external.thinking();
        }
        let waiting = !self.connected
            || self.pending.as_ref().is_some_and(|pending| {
                !pending.approvals.is_empty() || !pending.user_inputs.is_empty()
            });
        let tools_running = self.transcript.entries().iter().any(|entry| {
            entry
                .tool
                .as_ref()
                .is_some_and(|tool| super::tool_row::is_running(&tool.status))
        });
        !tools_running && self.feedback.thinking(&self.execution, waiting)
    }

    pub(super) fn activity(&self) -> Option<Activity<'_>> {
        if !self.connected {
            return None;
        }
        let canceling = self.external.as_ref().map_or_else(
            || {
                self.execution
                    .active
                    .as_ref()
                    .is_some_and(|run| run.status == AppRunStatus::CancelRequested)
            },
            |run| run.canceling,
        );
        if canceling {
            return Some(Activity {
                text: "Остановка…",
                animated: false,
            });
        }
        let waiting = self.pending.as_ref().is_some_and(|pending| {
            !pending.approvals.is_empty() || !pending.user_inputs.is_empty()
        }) || self.external.as_ref().is_some_and(|run| run.waiting);
        if waiting {
            return self.running().then_some(Activity {
                text: "Жду ответа",
                animated: false,
            });
        }
        let active = self.external.as_ref().map_or_else(
            || self.feedback.active(&self.execution),
            |run| run.running(),
        );
        if !active {
            return None;
        }
        let tool = self
            .transcript
            .entries()
            .iter()
            .rev()
            .filter_map(|entry| entry.tool.as_ref())
            .find(|tool| super::tool_row::is_running(&tool.status));
        Some(Activity {
            text: tool.map_or_else(
                || {
                    if self.feedback.preparing_tools() {
                        "Подготавливаю инструменты"
                    } else {
                        "Думаю"
                    }
                },
                |tool| tool.name.as_str(),
            ),
            animated: true,
        })
    }

    pub fn state(&self) -> ChatState {
        if !self.connected {
            if self.stream_closed {
                return ChatState::Trouble("Сессия закрыта");
            }
            if self.external.is_some() {
                return ChatState::Trouble("Codex отключён");
            }
            return match self.link_error {
                Some(_) => ChatState::Trouble("Переподключение…"),
                None => ChatState::Busy("Подключение…"),
            };
        }
        if let Some(external) = &self.external {
            return if external.running() {
                ChatState::Busy(if external.canceling {
                    "Остановка…"
                } else {
                    "Работает…"
                })
            } else {
                ChatState::Idle
            };
        }
        match self.execution.active.as_ref().map(|run| &run.status) {
            Some(AppRunStatus::CancelRequested) => ChatState::Busy("Остановка…"),
            Some(_) => ChatState::Busy("Работает…"),
            None => ChatState::Idle,
        }
    }
}

#[cfg(test)]
#[path = "feedback_routing_tests.rs"]
mod feedback_routing_tests;
