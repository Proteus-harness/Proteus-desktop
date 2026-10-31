//! Bound one ready batch and partition it without crossing any non-text event.
use crate::connection::PumpItem;
use crate::transcript::TextRun;

// Match the startup fold's event-count budget. This is not a payload-byte budget.
const MAX_READY_EVENTS: usize = 256;

pub(super) fn ready_batch(
    first: PumpItem,
    events: &async_channel::Receiver<PumpItem>,
) -> Vec<PumpItem> {
    let mut batch = vec![first];
    for _ in 1..MAX_READY_EVENTS {
        let Ok(item) = events.try_recv() else {
            break;
        };
        batch.push(item);
    }
    batch
}

/// A ready receiver must not keep the foreground task in one unbounded poll.
/// Requeue without a timer; the executor decides when other UI work runs.
pub(super) async fn yield_after_batch() {
    let mut yielded = false;
    std::future::poll_fn(move |cx| {
        if std::mem::replace(&mut yielded, true) {
            std::task::Poll::Ready(())
        } else {
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
    .await
}

pub(super) enum BatchItem {
    Text(TextRun),
    Barrier(PumpItem),
}

pub(super) struct Batches {
    items: std::iter::Peekable<std::vec::IntoIter<PumpItem>>,
}

pub(super) fn batches(items: Vec<PumpItem>) -> Batches {
    Batches {
        items: items.into_iter().peekable(),
    }
}

impl Iterator for Batches {
    type Item = BatchItem;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.items.next()?;
        let PumpItem::Event(event) = item else {
            return Some(BatchItem::Barrier(item));
        };
        let mut run = match TextRun::new(event) {
            Ok(run) => run,
            Err(event) => return Some(BatchItem::Barrier(PumpItem::Event(event))),
        };
        while matches!(self.items.peek(), Some(PumpItem::Event(event)) if run.accepts(event)) {
            let Some(PumpItem::Event(event)) = self.items.next() else {
                unreachable!("peeked text event");
            };
            run.push(event);
        }
        Some(BatchItem::Text(run))
    }
}

#[cfg(test)]
mod tests;
