//! Insertion and animation geometry, independent of widgets or packages.
use crate::ui::motion;
use gpui_kit::{Pixels, SharedString};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use std::time::Instant;

pub(super) struct Pending {
    pub source: Vec<SharedString>,
    pub order: Vec<SharedString>,
    pub revision: Option<u64>,
}

struct Position {
    from: Pixels,
    target: Pixels,
    tween: motion::Tween,
}

/// Only changes of relative row order start a FLIP transition. Ordinary
/// layout (a disclosure opening, wrapping text, an inserted row) owns its
/// own geometry and must not be followed by another delayed movement.
#[derive(Default)]
pub(super) struct Arrangement {
    order: Vec<SharedString>,
    positions: HashMap<SharedString, Position>,
}

impl Arrangement {
    pub fn layout(
        &mut self,
        order: Vec<SharedString>,
        heights: &HashMap<SharedString, Pixels>,
        gap: Pixels,
        now: Instant,
        animate: bool,
    ) -> (HashMap<SharedString, Pixels>, bool) {
        let reordered = self.order != order && retained_order_changed(&self.order, &order);
        self.positions.retain(|id, _| heights.contains_key(id));
        self.order = order;
        let mut top = Pixels::default();
        let mut shown = HashMap::with_capacity(self.order.len());
        let mut moving = false;
        for id in &self.order {
            let position = self
                .positions
                .entry(id.clone())
                .or_insert_with(|| Position {
                    from: top,
                    target: top,
                    tween: motion::Tween::start_at(Duration::ZERO, now),
                });
            if !animate {
                position.from = top;
                position.target = top;
                position.tween = motion::Tween::start_at(Duration::ZERO, now);
            } else if reordered && position.target != top {
                position.from = position.sample(now);
                position.target = top;
                position.tween = motion::Tween::start_at(motion::BASE, now);
            } else {
                // Translate an in-flight reorder together with fresh layout,
                // retaining its clock and displacement relative to that layout.
                position.from += top - position.target;
                position.target = top;
            }
            let y = position.sample(now);
            moving |= y != top;
            shown.insert(id.clone(), y);
            top += heights.get(id).copied().unwrap_or_default() + gap;
        }
        (shown, moving)
    }
}

fn retained_order_changed(previous: &[SharedString], next: &[SharedString]) -> bool {
    let old: HashSet<_> = previous.iter().collect();
    let new: HashSet<_> = next.iter().collect();
    previous
        .iter()
        .filter(|id| new.contains(id))
        .ne(next.iter().filter(|id| old.contains(id)))
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
    use super::{Arrangement, Pending, keyboard_target, move_before, preview_order};
    use crate::ui::motion;
    use gpui_kit::{Pixels, SharedString, px};
    use std::{collections::HashMap, time::Instant};

    fn ids(order: &[&str]) -> Vec<SharedString> {
        order.iter().map(|id| (*id).into()).collect()
    }

    fn heights(rows: &[(&str, f32)]) -> HashMap<SharedString, Pixels> {
        rows.iter()
            .map(|(id, height)| ((*id).into(), px(*height)))
            .collect()
    }

    #[::core::prelude::v1::test]
    fn same_order_follows_disclosure_height_without_a_second_animation() {
        let now = Instant::now();
        let mut arrangement = Arrangement::default();
        let order = ids(&["a", "b", "c"]);
        for (step, height) in [20., 70., 120., 45., 20.].into_iter().enumerate() {
            let at = now + motion::BASE * step as u32 / 4;
            let (shown, moving) = arrangement.layout(
                order.clone(),
                &heights(&[("a", height), ("b", 40.), ("c", 30.)]),
                px(8.),
                at,
                true,
            );
            assert_eq!(shown["a"], px(0.));
            assert_eq!(shown["b"], px(height + 8.));
            assert_eq!(shown["c"], px(height + 56.));
            assert!(!moving);
            assert_eq!(arrangement.positions["b"].tween.age(at), at - now);
        }
    }

    #[::core::prelude::v1::test]
    fn reflow_during_a_reorder_translates_both_endpoints_without_restarting_time() {
        let now = Instant::now();
        let mut arrangement = Arrangement::default();
        let initial = heights(&[("a", 20.), ("b", 40.), ("c", 30.)]);
        arrangement.layout(ids(&["a", "b", "c"]), &initial, px(8.), now, true);
        let order = ids(&["b", "a", "c"]);
        let (first, moving) = arrangement.layout(order.clone(), &initial, px(8.), now, true);
        assert_eq!(first["a"], px(0.));
        assert_eq!(first["b"], px(28.));
        assert!(moving);
        let middle = now + motion::BASE / 2;
        let before = arrangement.positions["a"].sample(middle);
        let grown = heights(&[("a", 20.), ("b", 80.), ("c", 30.)]);
        let (shown, moving) = arrangement.layout(order.clone(), &grown, px(8.), middle, true);
        assert_eq!(shown["a"], before + px(40.));
        assert_eq!(shown["c"], px(116.));
        assert_eq!(
            arrangement.positions["a"].tween.age(middle),
            motion::BASE / 2
        );
        assert!(moving);
        let before_reverse = shown.clone();
        let (reversed, moving) =
            arrangement.layout(ids(&["a", "b", "c"]), &grown, px(8.), middle, true);
        assert_eq!(reversed, before_reverse);
        assert!(moving);
        let (settled, moving) = arrangement.layout(
            ids(&["a", "b", "c"]),
            &grown,
            px(8.),
            middle + motion::BASE,
            true,
        );
        assert_eq!(settled["a"], px(0.));
        assert_eq!(settled["b"], px(28.));
        assert_eq!(settled["c"], px(116.));
        assert!(!moving);
    }

    #[::core::prelude::v1::test]
    fn inserted_and_removed_rows_follow_layout_instead_of_looking_like_reorders() {
        let now = Instant::now();
        let mut arrangement = Arrangement::default();
        let sizes = heights(&[("a", 20.), ("b", 40.), ("x", 10.)]);
        arrangement.layout(ids(&["a", "b"]), &sizes, px(8.), now, true);
        let (inserted, moving) =
            arrangement.layout(ids(&["x", "a", "b"]), &sizes, px(8.), now, true);
        assert_eq!(inserted["a"], px(18.));
        assert_eq!(inserted["b"], px(46.));
        assert!(!moving);
        let (removed, moving) = arrangement.layout(ids(&["x", "b"]), &sizes, px(8.), now, true);
        assert_eq!(removed["b"], px(18.));
        assert!(!moving);
    }

    #[::core::prelude::v1::test]
    fn reducing_or_disabling_motion_settles_without_resurrecting_old_positions() {
        use crate::preferences::MotionPreference::{Full, Off, Reduced};
        let now = Instant::now();
        let sizes = heights(&[("a", 20.), ("b", 40.)]);
        for stopped in [Reduced, Off] {
            let mut arrangement = Arrangement::default();
            arrangement.layout(ids(&["a", "b"]), &sizes, px(8.), now, true);
            arrangement.layout(ids(&["b", "a"]), &sizes, px(8.), now, true);
            for (index, mode) in [stopped, Full].into_iter().enumerate() {
                let (shown, moving) = arrangement.layout(
                    ids(&["b", "a"]),
                    &sizes,
                    px(8.),
                    now + motion::BASE / 4 * index as u32,
                    mode == Full,
                );
                assert_eq!(shown["a"], px(48.));
                assert_eq!(shown["b"], px(0.));
                assert!(!moving);
            }
        }
    }
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
