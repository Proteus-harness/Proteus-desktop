//! Coalesces context reads and fences their replies against newer stream state.

#[derive(Default)]
pub(super) struct ContextRequests {
    generation: u64,
    in_flight: Option<u64>,
    pending: bool,
}

pub(super) struct Completion {
    pub apply: bool,
    pub next: Option<u64>,
}

impl ContextRequests {
    /// A newer requested snapshot makes the outstanding response obsolete.
    pub fn request(&mut self) -> Option<u64> {
        self.invalidate();
        if self.in_flight.is_some() {
            self.pending = true;
            None
        } else {
            self.in_flight = Some(self.generation);
            Some(self.generation)
        }
    }

    /// Live usage is newer than any HTTP response already being fetched.
    pub fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    /// Called for success and failure; neither may leave the slot occupied.
    pub fn finish(&mut self, generation: u64) -> Completion {
        if self.in_flight != Some(generation) {
            return Completion {
                apply: false,
                next: None,
            };
        }
        let apply = self.generation == generation;
        self.in_flight = None;
        let next = if std::mem::take(&mut self.pending) {
            self.in_flight = Some(self.generation);
            Some(self.generation)
        } else {
            None
        };
        Completion { apply, next }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_refreshes_coalesce_and_only_latest_requested_state_may_apply() {
        let mut requests = ContextRequests::default();
        let initial = requests.request().unwrap();
        assert!(requests.request().is_none());
        assert!(requests.request().is_none());
        let completed = requests.finish(initial);
        assert!(!completed.apply);
        let follow_up = completed.next.unwrap();
        assert!(requests.in_flight.is_some());
        let completed = requests.finish(follow_up);
        assert!(completed.apply);
        assert!(completed.next.is_none());
        assert!(requests.in_flight.is_none());
    }

    #[test]
    fn live_usage_and_reconnect_fence_late_success_or_failure_without_losing_pending_refresh() {
        let mut requests = ContextRequests::default();
        let old = requests.request().unwrap();
        requests.invalidate();
        let completed = requests.finish(old);
        assert!(!completed.apply);
        assert!(completed.next.is_none());
        // A rejected failure has released the slot and cannot prevent a retry.
        let retry = requests.request().unwrap();
        requests.invalidate();
        assert!(requests.request().is_none());
        let pending = requests.finish(retry);
        assert!(!pending.apply);
        let current = pending.next.unwrap();
        // A duplicate old completion must not release the newer request slot.
        assert!(requests.finish(retry).next.is_none());
        assert_eq!(requests.in_flight, Some(current));
        assert!(requests.finish(current).apply);
    }
}
