//! Live rearranging while a tab is dragged: the tab moves into the strip it
//! hovers, where it would land, and its old place closes up behind it. Over
//! a pane's content or away from every strip it goes back to its place.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::*;

use crate::tabs::{Pane, PaneId, TabLayout};
use crate::ui::{drag, prefs};

/// How far above and below a strip the pointer still counts as over it.
const REACH_Y: Pixels = px(8.);

/// Where the dragged tab would land: a pane and an index among its other
/// tabs.
#[derive(Clone, Copy, Default, PartialEq)]
enum Target {
    #[default]
    Home,
    At(PaneId, usize),
}

#[derive(Default)]
struct Arrange {
    dragging: Option<String>,
    /// Bumped whenever a drag starts or ends, so the strips are drawn afresh
    /// instead of gliding between the dragged and the settled layout.
    generation: u64,
    target: Target,
    /// Last drawn bounds of every tab, for hit testing and the gap width.
    bounds: HashMap<String, Bounds<Pixels>>,
}

/// Shared by the strips of one workspace, never by two system windows.
#[derive(Clone, Default)]
pub(in crate::ui) struct TabArrangement(Rc<RefCell<Arrange>>);

impl TabArrangement {
    fn update<R>(&self, f: impl FnOnce(&mut Arrange) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }
}

/// How one strip is drawn this frame.
pub(super) struct Frame {
    /// The tab being dragged, if a tab is.
    pub dragging: Option<String>,
    /// Gap among the other tabs of this strip that stays open.
    pub open_gap: Option<usize>,
    pub generation: u64,
    /// Width of the dragged tab, the room a gap opens to.
    pub gap_width: Pixels,
}

pub(super) fn frame(
    arrangement: &TabArrangement,
    pane: &Pane,
    layout: &TabLayout,
    cx: &App,
) -> Frame {
    let dragging = drag::current(cx)
        .and_then(|key| key.strip_prefix("tab:"))
        .filter(|id| layout.contains(id))
        .map(str::to_string);
    arrangement.update(|state| {
        if state.dragging != dragging {
            if dragging.is_none() {
                state.bounds.clear();
            }
            state.dragging = dragging.clone();
            state.generation += 1;
            state.target = Target::Home;
        }
        let open_gap = match state.target {
            Target::At(at, index) if at == pane.id => Some(index),
            Target::At(..) => None,
            Target::Home => dragging
                .as_deref()
                .and_then(|id| pane.tabs.iter().position(|tab| tab == id)),
        };
        let gap_width = dragging
            .as_ref()
            .and_then(|id| state.bounds.get(id))
            .map_or(px(120. * prefs::scale(cx)), |bounds| bounds.size.width);
        Frame {
            dragging,
            open_gap,
            generation: state.generation,
            gap_width,
        }
    })
}

pub(super) fn record(arrangement: &TabArrangement, id: &str, bounds: Bounds<Pixels>) {
    arrangement.update(|state| match state.bounds.get_mut(id) {
        Some(known) => *known = bounds,
        None => {
            state.bounds.insert(id.to_string(), bounds);
        }
    });
}

/// Follows the pointer over the strip of `pane`: over it the tab goes in
/// front of the first other tab whose middle lies to the right, within its
/// own group of pinned or unpinned tabs; leaving the strip sends it back.
pub(super) fn hover(
    arrangement: &TabArrangement,
    pane: &Pane,
    dragging: &str,
    pinned: bool,
    strip: Bounds<Pixels>,
    position: Point<Pixels>,
    window: &mut Window,
) {
    let reach = strip.extend(Edges {
        top: REACH_Y,
        bottom: REACH_Y,
        ..Default::default()
    });
    arrangement.update(|state| {
        let next = if reach.contains(&position) {
            let others = others(pane, dragging);
            let index = others
                .iter()
                .filter(|id| {
                    state
                        .bounds
                        .get(id.as_str())
                        .is_some_and(|bounds| bounds.center().x < position.x)
                })
                .count();
            Target::At(pane.id, clamp(pane, dragging, pinned, index))
        } else {
            match state.target {
                Target::At(at, _) if at == pane.id => Target::Home,
                _ => return,
            }
        };
        if state.target != next {
            state.target = next;
            window.refresh();
        }
    });
}

/// The tab a drop on the strip of `pane` lands in front of: `Some(None)`
/// for the end of its group, `None` when the drag is not over this strip.
pub(super) fn drop_before(
    arrangement: &TabArrangement,
    pane: &Pane,
    dragging: &str,
) -> Option<Option<String>> {
    let index = match arrangement.0.borrow().target {
        Target::At(at, index) if at == pane.id => index,
        _ => return None,
    };
    Some(others(pane, dragging).get(index).cloned())
}

/// The tabs of `pane` other than the dragged one, in order.
pub(super) fn others(pane: &Pane, dragging: &str) -> Vec<String> {
    pane.tabs
        .iter()
        .filter(|tab| *tab != dragging)
        .cloned()
        .collect()
}

/// Keeps a pinned tab among the pinned ones and an unpinned one after them,
/// where the layout would put it.
fn clamp(pane: &Pane, dragging: &str, pinned: bool, index: usize) -> usize {
    let pinned_others = pane.tabs[..pane.pinned]
        .iter()
        .filter(|tab| *tab != dragging)
        .count();
    let others = pane.tabs.len() - usize::from(pane.tabs.iter().any(|tab| tab == dragging));
    if pinned {
        index.min(pinned_others)
    } else {
        index.clamp(pinned_others, others)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn another_windows_frame_keeps_the_first_windows_target_and_geometry() {
        let app = ::gpui::TestAppContext::single();
        let first = TabArrangement::default();
        let second = TabArrangement::default();
        let mut pane = Pane::new(PaneId(0));
        pane.tabs = vec!["a".into(), "b".into(), "c".into()];
        record(
            &first,
            "a",
            Bounds::new(point(px(10.), px(20.)), size(px(80.), px(24.))),
        );
        first.update(|state| {
            state.dragging = Some("a".into());
            state.target = Target::At(pane.id, 1);
        });
        record(
            &second,
            "a",
            Bounds::new(point(px(400.), px(90.)), size(px(160.), px(24.))),
        );
        app.update(|cx| {
            prefs::init_in_memory(cx);
            frame(&second, &pane, &TabLayout::default(), cx);
        });
        assert_eq!(drop_before(&first, &pane, "a"), Some(Some("c".into())));
        assert_eq!(first.0.borrow().bounds["a"].size.width, px(80.));
        assert!(drop_before(&second, &pane, "a").is_none());
    }
}
