//! Layout effects of one changed row, shared by the pump and parity tests.
use std::ops::Range;

use crate::transcript::{Change, Entry, affected_rows};

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
