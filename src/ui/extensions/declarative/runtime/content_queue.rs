//! One in-flight source revision and one replaceable latest source. The
//! caller owns the latest payload; this gate never retains obsolete text.

#[derive(Default)]
pub(super) struct ContentQueue {
    inflight: Option<Option<u64>>,
    pending: bool,
}

impl ContentQueue {
    /// Materialize an owned wire payload only for an admitted request. The
    /// caller keeps its shared latest snapshot when an intermediate edit waits.
    pub(super) fn begin_with<T>(
        &mut self,
        revision: Option<u64>,
        materialize: impl FnOnce() -> T,
    ) -> Option<T> {
        self.begin(revision).then(materialize)
    }

    fn begin(&mut self, revision: Option<u64>) -> bool {
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
    use crate::declarative::protocol::Content;
    use std::{cell::Cell, sync::Arc};

    fn snapshot(revision: u64) -> Arc<Content> {
        Arc::new(Content {
            language: "fixture".into(),
            source: format!("revision {revision}\n{}", "αβ😀\n".repeat(1024)),
            context: serde_json::json!({"custom": "preserved"}),
            source_revision: Some(revision),
        })
    }

    #[test]
    fn coalesced_sources_are_shared_and_only_admitted_payloads_are_materialized() {
        let mut queue = ContentQueue::default();
        let materializations = Cell::new(0);
        let materialize = |source: &Content| {
            materializations.set(materializations.get() + 1);
            source.clone()
        };
        let mut latest = snapshot(1);
        let first = queue
            .begin_with(latest.source_revision, || materialize(&latest))
            .unwrap();
        assert_eq!(first, *latest);

        for revision in 2..=300 {
            let obsolete = Arc::downgrade(&latest);
            latest = snapshot(revision);
            assert!(obsolete.upgrade().is_none(), "obsolete source was retained");
            let shell = Arc::clone(&latest);
            assert!(Arc::ptr_eq(&shell, &latest));
            assert!(
                queue
                    .begin_with(latest.source_revision, || materialize(&latest))
                    .is_none()
            );
        }
        assert_eq!(materializations.get(), 1);
        assert!(!queue.finish(Some(0), latest.source_revision));
        assert!(queue.finish(first.source_revision, latest.source_revision));
        let final_payload = queue
            .begin_with(latest.source_revision, || materialize(&latest))
            .unwrap();
        assert_eq!(final_payload, *latest);
        assert_eq!(final_payload.source_revision, Some(300));
        assert_eq!(materializations.get(), 2);
        assert!(!queue.finish(first.source_revision, latest.source_revision));
        assert!(!queue.finish(final_payload.source_revision, latest.source_revision));
    }

    #[test]
    fn reset_admits_the_retained_latest_source_and_drops_pending_work() {
        let mut queue = ContentQueue::default();
        let first = snapshot(1);
        assert!(
            queue
                .begin_with(first.source_revision, || first.as_ref().clone())
                .is_some()
        );
        let latest = snapshot(2);
        assert!(
            queue
                .begin_with(latest.source_revision, || panic!(
                    "busy queue materialized source"
                ))
                .is_none()
        );
        queue.reset();
        let restarted = queue
            .begin_with(latest.source_revision, || latest.as_ref().clone())
            .unwrap();
        assert_eq!(restarted, *latest);
        assert!(!queue.finish(first.source_revision, latest.source_revision));
        assert!(!queue.finish(restarted.source_revision, latest.source_revision));
    }

    #[test]
    fn absent_payload_is_an_admitted_request_not_a_busy_queue() {
        let mut queue = ContentQueue::default();
        assert_eq!(queue.begin_with(None, || None::<Content>), Some(None));
        assert!(queue.begin_with(None, || None::<Content>).is_none());
        assert!(queue.finish(None, None));
        assert_eq!(queue.begin_with(None, || None::<Content>), Some(None));
        assert!(!queue.finish(None, None));
    }

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

    #[test]
    fn invalidation_coalesces_without_a_source_revision_change_or_queue_reset() {
        let mut queue = ContentQueue::default();
        assert!(queue.begin(Some(7)));
        for _ in 0..100 {
            assert!(!queue.begin(Some(7)));
        }
        assert!(!queue.finish(Some(6), Some(7)));
        assert!(
            queue.finish(Some(7), Some(7)),
            "settings invalidation must refresh even when the source is unchanged"
        );
        assert!(queue.begin(Some(7)));
        assert!(!queue.finish(Some(7), Some(7)));

        assert!(queue.begin(Some(7)));
        assert!(!queue.begin(Some(8)));
        assert!(!queue.begin(Some(8)));
        assert!(queue.finish(Some(7), Some(8)));
        assert!(queue.begin(Some(8)));
        assert!(!queue.finish(Some(8), Some(8)));
    }
}
