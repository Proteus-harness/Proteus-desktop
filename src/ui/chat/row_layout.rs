//! Layout effects of one changed row, shared by the pump and parity tests.
use std::ops::Range;

use super::history::Direction;
use crate::transcript::{Change, Entry, affected_rows};

#[derive(Default)]
pub(super) struct RowContext {
    leading: Range<usize>,
    trailing: Range<usize>,
    footer: Range<usize>,
}

impl RowContext {
    pub(super) fn missing(&self, row: usize) -> Option<Direction> {
        if self.leading.contains(&row) {
            Some(Direction::Older)
        } else if self.trailing.contains(&row) || self.footer.contains(&row) {
            Some(Direction::Newer)
        } else {
            None
        }
    }

    pub(super) fn incomplete_rows(self) -> Vec<Range<usize>> {
        [self.leading, self.trailing, self.footer]
            .into_iter()
            .filter(|rows| !rows.is_empty())
            .collect()
    }
}

/// A page can clip a whole semantic group, including members which are not
/// raw edge rows. Their temporary layout must not replace document heights.
pub(super) fn row_context(
    entries: &[Entry],
    compact: bool,
    range: Option<super::scroller::HistoryRange>,
) -> RowContext {
    let Some(range) = range.filter(|_| !entries.is_empty()) else {
        return RowContext::default();
    };
    let mut context = RowContext::default();
    if range.start > 0 {
        context.leading = if compact {
            crate::transcript::tool_run(entries, 0).unwrap_or(0..1)
        } else {
            0..1
        };
    }
    if range.start + range.rows < range.total {
        let last = entries.len() - 1;
        context.trailing = if compact {
            crate::transcript::tool_run(entries, last).unwrap_or(last..last + 1)
        } else {
            last..last + 1
        };
        // The footer belongs to the last visible row (or its folded head),
        // even if duplicate child-progress rows form an invisible suffix.
        if let Some(last) = entries
            .iter()
            .rposition(|entry| !super::stamp::hidden(entries, entry))
        {
            context.footer = if compact {
                crate::transcript::tool_chain(entries, last).unwrap_or(last..last + 1)
            } else {
                last..last + 1
            };
        }
    }
    context
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct RowLayout {
    pub primary: Change,
    pub fresh_key: Option<u64>,
    pub chain: Option<Range<usize>>,
}

/// Only splices reconcile retained row state; live text needs no snapshot pass.
pub(super) fn reconciles_rows(change: Change) -> bool {
    matches!(change, Change::Spliced { .. })
}

pub(super) fn row_layout(entries: &[Entry], change: Change) -> Option<RowLayout> {
    let row = match change {
        Change::Pushed => entries.len().checked_sub(1),
        Change::Updated(row) => Some(row),
        Change::None | Change::Spliced { .. } => return None,
    };
    let fresh_key = (change == Change::Pushed)
        .then(|| entries.last().map(|entry| entry.key))
        .flatten();
    let chain = row
        .map(|row| affected_rows(entries, row))
        .filter(|rows| rows.len() > 1);
    Some(RowLayout {
        primary: change,
        fresh_key,
        chain,
    })
}
