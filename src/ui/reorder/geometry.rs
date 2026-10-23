//! Insertion and animation geometry, independent of widgets or packages.
use crate::ui::motion;
use gpui_kit::{Pixels, SharedString};
use std::time::Instant;

pub(super) struct Pending {
    pub source: Vec<SharedString>,
    pub order: Vec<SharedString>,
    pub revision: Option<u64>,
}

pub(super) struct Position {
    pub from: Pixels,
    pub target: Pixels,
    pub tween: motion::Tween,
}

impl Pending {
    pub fn retained(&self, ids: &[SharedString], revision: Option<u64>) -> bool {
        self.source == ids && self.order != ids && (revision.is_none() || revision == self.revision)
    }
}

pub(super) fn keyboard_target(
    ids: &[SharedString],
    id: &SharedString,
    down: bool,
) -> Option<Option<SharedString>> {
    let at = ids.iter().position(|candidate| candidate == id)?;
    if down && at + 1 < ids.len() {
        Some(ids.get(at + 2).cloned())
    } else if !down && at > 0 {
        Some(Some(ids[at - 1].clone()))
    } else {
        None
    }
}

impl Position {
    pub fn sample(&self, now: Instant) -> Pixels {
        self.from + (self.target - self.from) * motion::ease_enter(self.tween.linear(now))
    }
}

/// Moves an identity before another one, or to the end. Unknown identities
/// are a no-op; the target is resolved after removal, so moving downward
/// cannot accidentally skip a row.
pub(crate) fn move_before<T: Clone + PartialEq>(
    rows: &mut Vec<T>,
    id: &T,
    before: Option<&T>,
) -> bool {
    let Some(from) = rows.iter().position(|row| row == id) else {
        return false;
    };
    if before == Some(id) {
        return false;
    }
    if before.is_some_and(|before| !rows.contains(before)) {
        return false;
    }
    let row = rows.remove(from);
    let at = before
        .and_then(|before| rows.iter().position(|row| row == before))
        .unwrap_or(rows.len());
    rows.insert(at, row);
    at != from
}

pub(super) fn preview_order(
    ids: &[SharedString],
    dragged: Option<&SharedString>,
    before: Option<&SharedString>,
) -> Vec<SharedString> {
    let Some(dragged) = dragged else {
        return ids.to_vec();
    };
    let mut rows: Vec<_> = ids.iter().filter(|id| *id != dragged).cloned().collect();
    let at = before
        .and_then(|before| rows.iter().position(|id| id == before))
        .unwrap_or(rows.len());
    rows.insert(at, dragged.clone());
    rows
}

#[cfg(test)]
mod tests {
    use super::{Pending, keyboard_target, move_before, preview_order};
    use gpui_kit::SharedString;
    #[::core::prelude::v1::test]
    fn pending_order_waits_for_the_next_authoritative_tree_and_can_roll_back() {
        let source: Vec<SharedString> = ["a", "b"].into_iter().map(Into::into).collect();
        let order = vec![source[1].clone(), source[0].clone()];
        let pending = Pending {
            source: source.clone(),
            order: order.clone(),
            revision: Some(2),
        };
        assert!(pending.retained(&source, Some(2)));
        assert!(!pending.retained(&source, Some(3)));
        assert!(!pending.retained(&order, Some(3)));
        assert_eq!(keyboard_target(&source, &source[0], false), None);
        assert_eq!(keyboard_target(&source, &source[0], true), Some(None));
        assert_eq!(
            keyboard_target(&source, &source[1], false),
            Some(Some(source[0].clone()))
        );
    }

    #[::core::prelude::v1::test]
    fn moving_down_resolves_the_target_after_removal_and_handles_cancel_targets() {
        let mut rows = vec!["a", "b", "c", "d"];
        assert!(move_before(&mut rows, &"a", Some(&"c")));
        assert_eq!(rows, ["b", "a", "c", "d"]);
        assert!(move_before(&mut rows, &"a", None));
        assert_eq!(rows, ["b", "c", "d", "a"]);
        let before = rows.clone();
        for (id, target) in [("a", Some("a")), ("missing", None), ("a", Some("gone"))] {
            assert!(!move_before(&mut rows, &id, target.as_ref()));
            assert_eq!(rows, before);
        }
    }

    #[::core::prelude::v1::test]
    fn insertion_preview_is_stable_and_can_reach_both_ends() {
        let ids: Vec<SharedString> = ["a", "b", "c"].into_iter().map(Into::into).collect();
        assert_eq!(
            preview_order(&ids, Some(&ids[0]), None),
            [ids[1].clone(), ids[2].clone(), ids[0].clone()]
        );
        assert_eq!(
            preview_order(&ids, Some(&ids[2]), Some(&ids[0])),
            [ids[2].clone(), ids[0].clone(), ids[1].clone()]
        );
        assert_eq!(preview_order(&ids, None, None), ids);
    }
}
