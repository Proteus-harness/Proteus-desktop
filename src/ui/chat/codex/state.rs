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
    pub fn thinking(&self) -> bool {
        self.running() && !self.output && !self.waiting && !self.canceling
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
        self.starting = false;
        if self.turn.as_ref() != Some(&id) {
            self.turn = Some(id);
            self.canceling = false;
        }
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
        true
    }
    pub fn failed_send(&mut self) {
        if self.starting {
            self.starting = false;
        }
    }
    pub fn disconnected(&mut self) {
        self.turn = None;
        self.starting = false;
        self.canceling = false;
        self.waiting = false;
        self.output = false;
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
    fn request_and_output_suppress_thinking() {
        let mut state = RunState::default();
        state.begin();
        assert!(state.thinking());
        state.waiting = true;
        assert!(!state.thinking());
        state.waiting = false;
        state.output = true;
        assert!(!state.thinking());
    }
}
