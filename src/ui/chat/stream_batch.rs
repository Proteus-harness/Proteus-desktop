//! Partition one ready batch without crossing any non-text event.
use crate::connection::PumpItem;
use crate::transcript::TextRun;

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
