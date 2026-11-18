//! Live rearranging while an icon is dragged: the icon moves into the row
//! it hovers, where it would land, and its old place closes up behind it.
//! Away from every row it goes back to its place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::*;

use crate::preferences::BadgePlace;
use crate::ui::extensions;
use crate::ui::{drag, prefs};

/// How far around a row the pointer still counts as over it, so a drop
/// that misses the icons by a little still lands.
const REACH_ALONG: Pixels = px(20.);
const REACH_ACROSS: Pixels = px(14.);
/// How far the drawn drop zone of a row reaches past it; less than the
/// reach, so the zone stays inside the bars.
const ZONE_ALONG: Pixels = px(3.);
const ZONE_ACROSS: Pixels = px(2.);

/// Where the dragged icon would land.
#[derive(Clone, Copy, Default, PartialEq)]
enum Target {
    /// Away from every row the icon holds its own place.
    #[default]
    Home,
    At(BadgePlace, usize),
}

#[derive(Default)]
struct Arrange {
    dragging: Option<&'static str>,
    /// Bumped whenever a drag starts or ends, so the rows are drawn afresh
    /// instead of gliding between the dragged and the settled layout.
    generation: u64,
    target: Target,
    /// Last drawn bounds of every icon, for hit testing and the gap width.
    bounds: HashMap<&'static str, Bounds<Pixels>>,
}

/// Window-local strip state. Callbacks of that window share the same handle;
/// another workspace can paint the same badge IDs at unrelated coordinates.
#[derive(Clone, Default)]
pub(in crate::ui) struct BadgeArrangement(Rc<RefCell<Arrange>>);

impl BadgeArrangement {
    fn update<R>(&self, update: impl FnOnce(&mut Arrange) -> R) -> R {
        update(&mut self.0.borrow_mut())
    }
}

/// How one row is drawn this frame.
pub(super) struct Frame {
    /// The icon being dragged, if any icon is and it fits this row.
    pub dragging: Option<&'static str>,
    /// Gap among the other icons of this row that stays open.
    pub open_gap: Option<usize>,
    /// The dragged icon would land in this row.
    pub hovered: bool,
    pub generation: u64,
    /// Length of the dragged icon along the row, the room a gap opens to.
    pub gap_length: Pixels,
}

pub(super) fn frame(
    arrangement: &BadgeArrangement,
    place: BadgePlace,
    ids: &[&'static str],
    cx: &App,
) -> Frame {
    let dragging = drag::current(cx)
        .and_then(|key| key.strip_prefix("badge:"))
        .and_then(|id| {
            extensions::badge_ids()
                .into_iter()
                .find(|known| *known == id)
        });
    arrangement.update(|state| {
        if state.dragging != dragging {
            state.dragging = dragging;
            state.generation += 1;
            state.target = Target::Home;
        }
        let dragging = dragging.filter(|id| fits(place, id));
        let open_gap = match state.target {
            Target::At(at, index) if at == place => Some(index),
            Target::At(..) => None,
            Target::Home => dragging.and_then(|id| ids.iter().position(|known| *known == id)),
        };
        let hovered = matches!(state.target, Target::At(at, _) if at == place);
        let gap_length = dragging.and_then(|id| state.bounds.get(id)).map_or(
            px(28. * prefs::scale(cx)),
            |bounds| {
                if place.is_rail() {
                    bounds.size.height
                } else {
                    bounds.size.width
                }
            },
        );
        Frame {
            dragging,
            open_gap,
            hovered,
            generation: state.generation,
            gap_length,
        }
    })
}

fn fits(place: BadgePlace, id: &str) -> bool {
    extensions::find(id).is_some_and(|manifest| extensions::fits(place, manifest))
}

/// How far past a row, sideways and up or down, the pointer still counts as
/// over it.
pub(super) fn reach(place: BadgePlace) -> (Pixels, Pixels) {
    if place.is_rail() {
        (REACH_ACROSS, REACH_ALONG)
    } else {
        (REACH_ALONG, REACH_ACROSS)
    }
}

/// How far past a row, sideways and up or down, its drop zone is drawn.
pub(super) fn zone(place: BadgePlace) -> (Pixels, Pixels) {
    if place.is_rail() {
        (ZONE_ACROSS, ZONE_ALONG)
    } else {
        (ZONE_ALONG, ZONE_ACROSS)
    }
}

pub(super) fn record(arrangement: &BadgeArrangement, id: &'static str, bounds: Bounds<Pixels>) {
    arrangement.0.borrow_mut().bounds.insert(id, bounds);
}

impl Arrange {
    fn insertion_index(
        &self,
        place: BadgePlace,
        others: &[&'static str],
        position: Point<Pixels>,
    ) -> usize {
        let along = |point: Point<Pixels>| if place.is_rail() { point.y } else { point.x };
        others
            .iter()
            .filter(|id| {
                self.bounds
                    .get(*id)
                    .is_some_and(|bounds| along(bounds.center()) < along(position))
            })
            .count()
    }
}

/// Follows the pointer over a row: within reach of it the icon goes in front
/// of the first other icon whose middle lies past the pointer; leaving the
/// row sends it back to its place. A row the icon does not fit ignores it.
pub(super) fn hover(
    arrangement: &BadgeArrangement,
    place: BadgePlace,
    others: &[&'static str],
    row: Bounds<Pixels>,
    position: Point<Pixels>,
    window: &mut Window,
) {
    let (x, y) = reach(place);
    let reach = row.extend(Edges {
        top: y,
        right: x,
        bottom: y,
        left: x,
    });
    arrangement.update(|state| {
        let welcome = state.dragging.is_some_and(|id| fits(place, id));
        let next = if welcome && reach.contains(&position) {
            Target::At(place, state.insertion_index(place, others, position))
        } else {
            match state.target {
                Target::At(at, _) if at == place => Target::Home,
                _ => return,
            }
        };
        if state.target != next {
            state.target = next;
            window.refresh();
        }
    });
}

/// Index among the other icons of `place` where a drop lands.
pub(super) fn drop_index(arrangement: &BadgeArrangement, place: BadgePlace) -> Option<usize> {
    match arrangement.0.borrow().target {
        Target::At(at, index) if at == place => Some(index),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn painting_the_same_badges_in_a_second_workspace_keeps_the_first_drop_position() {
        let a = BadgeArrangement::default();
        let b = BadgeArrangement::default();
        let a_strip = a.clone();
        for (owner, left) in [(&a_strip, 20.), (&b, 200.)] {
            record(
                owner,
                "b",
                Bounds::new(point(px(left), px(0.)), size(px(20.), px(20.))),
            );
            record(
                owner,
                "c",
                Bounds::new(point(px(left + 40.), px(0.)), size(px(20.), px(20.))),
            );
        }
        let position = point(px(50.), px(10.));
        assert_eq!(
            a.0.borrow()
                .insertion_index(BadgePlace::HeaderStart, &["b", "c"], position),
            1
        );
        assert_eq!(
            b.0.borrow()
                .insertion_index(BadgePlace::HeaderStart, &["b", "c"], position),
            0
        );
    }
}
