//! One in-flight source revision and one replaceable latest source. The
//! caller owns the latest payload; this gate never retains obsolete text.

#[derive(Default)]
pub(super) struct ContentQueue {
    inflight: Option<Option<u64>>,
    pending: bool,
}

impl ContentQueue {
    pub(super) fn begin(&mut self, revision: Option<u64>) -> bool {
        if self.inflight.is_some() {
            self.pending = true;
            return false;
        }
        self.inflight = Some(revision);
        true
    }
    /// Both success and content_error acknowledge the in-flight request.
    /// A late response to an already completed source cannot release a new one.
    pub(super) fn finish(&mut self, reply: Option<u64>, latest: Option<u64>) -> bool {
        if self.inflight != Some(reply) {
            return false;
        }
        self.inflight = None;
        std::mem::take(&mut self.pending) || latest != reply
    }
    pub(super) fn reset(&mut self) {
        self.inflight = None;
        self.pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::ContentQueue;
    #[test]
    fn slow_render_coalesces_hundreds_of_edits_to_the_latest_source() {
        let mut queue = ContentQueue::default();
        assert!(queue.begin(Some(1)));
        let mut latest = 1;
        for revision in 2..=300 {
            latest = revision;
            assert!(!queue.begin(Some(revision)));
        }
        assert!(queue.finish(Some(1), Some(latest)));
        assert!(queue.begin(Some(latest)));
        assert!(
            !queue.finish(Some(1), Some(latest)),
            "old reply cannot release revision 300"
        );
        assert!(!queue.begin(Some(301)));
        assert!(queue.finish(Some(300), Some(301)));
        assert!(queue.begin(Some(301)));
    }
    #[test]
    fn declined_or_restarted_source_does_not_block_the_next_render() {
        let mut queue = ContentQueue::default();
        assert!(queue.begin(None));
        assert!(queue.finish(None, Some(2)));
        assert!(queue.begin(Some(2)));
        queue.reset();
        assert!(queue.begin(Some(3)));
    }
}
