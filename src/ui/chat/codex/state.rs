//! Turn identity is owned by Codex. Old completions never settle a newer run.
use std::collections::VecDeque;

#[derive(Default)]
pub(in crate::ui::chat) struct RunState {
    pub turn: Option<String>,
    pub starting: bool,
    pub canceling: bool,
    pub waiting: bool,
    pub output: bool,
    finished: VecDeque<String>,
    interrupt_sent: Option<String>,
}
impl RunState {
    pub fn resumed(turn: Option<String>) -> Self {
        let mut state = Self::default();
        if let Some(turn) = turn.filter(|turn| !turn.is_empty()) {
            state.started(turn);
        }
        state
    }
    pub fn running(&self) -> bool {
        self.starting || self.turn.is_some()
    }
    #[cfg(test)]
    pub fn thinking(&self) -> bool {
        self.running() && !self.waiting && !self.canceling
    }
    pub fn begin(&mut self) -> bool {
        if self.running() {
            return false;
        }
        self.starting = true;
        self.canceling = false;
        self.output = false;
        true
    }
    pub fn started(&mut self, id: String) {
        if self.finished.contains(&id) {
            return;
        }
        if self.turn.as_ref().is_some_and(|turn| turn != &id) {
            return;
        }
        let cancel_requested = self.starting && self.canceling;
        self.starting = false;
        if self.turn.as_ref() != Some(&id) {
            self.turn = Some(id);
            self.canceling = cancel_requested;
        }
    }
    pub fn request_cancel(&mut self) {
        if self.running() {
            self.canceling = true;
        }
    }
    pub fn acknowledged(&mut self, id: String) {
        if self.finished.contains(&id) && self.turn.is_none() {
            self.disconnected();
        } else {
            self.started(id);
        }
    }
    pub fn take_interrupt(&mut self) -> Option<String> {
        let turn = self.turn.as_ref()?;
        if !self.canceling || self.interrupt_sent.as_ref() == Some(turn) {
            return None;
        }
        self.interrupt_sent = Some(turn.clone());
        Some(turn.clone())
    }
    pub fn interrupt_failed(&mut self, turn: &str) -> bool {
        if self.turn.as_deref() != Some(turn) {
            return false;
        }
        self.canceling = false;
        self.interrupt_sent = None;
        true
    }
    pub fn completed(&mut self, id: &str) -> bool {
        if !self.finished.iter().any(|old| old == id) {
            self.finished.push_back(id.into());
            if self.finished.len() > 32 {
                self.finished.pop_front();
            }
        }
        if self.turn.as_deref() != Some(id) {
            return false;
        }
        self.turn = None;
        self.starting = false;
        self.canceling = false;
        self.output = false;
        self.waiting = false;
        self.interrupt_sent = None;
        true
    }
    pub fn failed_send(&mut self) {
        if self.starting {
            self.starting = false;
            self.canceling = false;
        }
    }
    pub fn disconnected(&mut self) {
        self.turn = None;
        self.starting = false;
        self.canceling = false;
        self.waiting = false;
        self.output = false;
        self.interrupt_sent = None;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn stale_completion_and_ack_do_not_resurrect_or_settle_runs() {
        let mut state = RunState::default();
        assert!(state.begin());
        assert!(!state.begin());
        state.started("opaque:A".into());
        assert!(state.completed("opaque:A"));
        state.started("opaque:A".into());
        assert!(!state.running());
        state.begin();
        state.started("opaque:B".into());
        assert!(!state.completed("opaque:A"));
        assert_eq!(state.turn.as_deref(), Some("opaque:B"));
        state.canceling = true;
        assert!(!state.thinking());
        state.disconnected();
        assert!(!state.running());
    }
    #[::core::prelude::v1::test]
    fn completion_before_resume_construction_cannot_resurrect_busy_metadata() {
        // Summary.active may be stale by the time history RPC finishes. The
        // driver's routed current-turn identity is the authoritative seed.
        let mut state = RunState::resumed(None);
        assert!(!state.running());
        assert!(!state.completed("finished-during-resume"));
        assert!(!state.running());
        let mut state = RunState::resumed(Some("live:opaque".into()));
        assert!(state.completed("live:opaque"));
        assert!(!state.running());
    }
    #[::core::prelude::v1::test]
    fn completed_before_start_ack_does_not_reanimate_or_retain_cancel() {
        let mut state = RunState::default();
        state.begin();
        state.request_cancel();
        state.completed("fast");
        state.acknowledged("fast".into());
        assert!(!state.running());
        assert!(!state.canceling);
        assert!(state.take_interrupt().is_none());
        assert!(state.begin());
        state.acknowledged("new".into());
        assert!(state.take_interrupt().is_none());
    }
    #[::core::prelude::v1::test]
    fn request_pauses_activity_but_streamed_output_does_not_finish_it() {
        let mut state = RunState::default();
        state.begin();
        assert!(state.thinking());
        state.waiting = true;
        assert!(!state.thinking());
        state.waiting = false;
        state.output = true;
        assert!(state.thinking());
    }
    #[::core::prelude::v1::test]
    fn stop_before_start_is_delivered_once_and_cannot_leak_into_next_turn() {
        let mut state = RunState::default();
        state.begin();
        state.request_cancel();
        assert!(state.take_interrupt().is_none());
        state.started("A".into());
        assert_eq!(state.take_interrupt().as_deref(), Some("A"));
        state.started("A".into());
        assert!(state.take_interrupt().is_none());
        state.completed("A");
        state.begin();
        state.started("B".into());
        assert!(state.take_interrupt().is_none());
        assert!(!state.interrupt_failed("A"));
        assert_eq!(state.turn.as_deref(), Some("B"));
    }
}
