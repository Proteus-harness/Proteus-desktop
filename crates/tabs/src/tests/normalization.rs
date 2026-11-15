use super::super::tree::{Child, Split};
use super::super::{Axis, Edge, Node, Pane, PaneId, Size, TabLayout};
use super::tree::assert_close;

fn leaf(id: u32, tabs: &[&str], size: Size, locked: bool) -> Child {
    let mut pane = Pane::new(PaneId(id));
    pane.tabs = tabs.iter().map(|tab| (*tab).to_owned()).collect();
    pane.active = pane.tabs.first().cloned();
    pane.locked = locked;
    Child {
        size,
        node: Node::Pane(pane),
    }
}

fn split(axis: Axis, size: Size, children: Vec<Child>) -> Child {
    Child {
        size,
        node: Node::Split(Split { axis, children }),
    }
}

fn holder(locked: u32, empty: u32, tab: &str) -> Child {
    split(
        Axis::Column,
        Size::Weight(1.),
        vec![
            leaf(locked, &[tab], Size::Fixed(100.), true),
            leaf(empty, &[], Size::Weight(1.), false),
        ],
    )
}

fn wrapped(locked: u32, empty: u32, removed: u32, tab: &str, size: Size) -> Child {
    split(
        Axis::Row,
        size,
        vec![
            holder(locked, empty, tab),
            leaf(removed, &["remove:wrapper"], Size::Weight(1.), false),
        ],
    )
}

fn layout(children: Vec<Child>, focused: u32) -> TabLayout {
    TabLayout {
        root: Node::Split(Split {
            axis: Axis::Column,
            children,
        }),
        focused: PaneId(focused),
        next_id: 100,
        ..TabLayout::default()
    }
}

fn pane_order(layout: &TabLayout) -> Vec<PaneId> {
    layout.panes().into_iter().map(|pane| pane.id).collect()
}

fn fixed_size(layout: &TabLayout, pane: PaneId) -> Size {
    let (siblings, ix) = layout.siblings(pane).unwrap();
    siblings[ix].size
}

fn round_trip_and_settled(layout: &mut TabLayout) {
    assert_eq!(layout.validate(), Ok(()));
    let before = serde_json::to_value(&*layout).unwrap();
    let restored: TabLayout = serde_json::from_value(before.clone()).unwrap();
    assert_eq!(restored.validate(), Ok(()));
    assert_eq!(serde_json::to_value(restored).unwrap(), before);
    // Idempotence of these fixtures, not a universal floating-point claim.
    layout.retain(|_| true);
    assert_eq!(serde_json::to_value(&*layout).unwrap(), before);
}

#[test]
fn closing_nested_wrapper_reconciles_room_after_merge_in_both_axes_and_directions() {
    for (edge, cross) in [
        (Edge::Bottom, Edge::Right),
        (Edge::Top, Edge::Left),
        (Edge::Right, Edge::Bottom),
        (Edge::Left, Edge::Top),
    ] {
        let mut layout = TabLayout::default();
        for tab in ["files:a", "files:b", "files:c", "files:d"] {
            layout.open(tab, |_| false);
        }
        let a = layout.pane_of("files:a").unwrap();
        layout.split("files:b", a, edge);
        let b = layout.pane_of("files:b").unwrap();
        layout.split("files:c", b, cross);
        layout.split("files:d", b, edge);
        layout.lock_pane(b, true, 1200., 900., 5.);
        layout.close("files:d");
        assert_eq!(layout.validate(), Ok(()));
        let before_order: Vec<_> = pane_order(&layout)
            .into_iter()
            .filter(|id| *id == a || *id == b)
            .collect();
        let size = fixed_size(&layout, b);
        let before = layout
            .rects_spaced(1200., 900., 5.)
            .into_iter()
            .find(|(id, _)| *id == b)
            .unwrap()
            .1;
        layout.focus(b);
        layout.close("files:c");
        assert_eq!(layout.validate(), Ok(()));
        assert_eq!(pane_order(&layout), before_order);
        assert_eq!(layout.focused, b);
        assert_eq!(layout.pane_locked(b), Some(true));
        assert_eq!(fixed_size(&layout, b), size);
        let after = layout
            .rects_spaced(1200., 900., 5.)
            .into_iter()
            .find(|(id, _)| *id == b)
            .unwrap()
            .1;
        match edge.axis() {
            Axis::Column => assert_close(&[after.h], &[before.h]),
            Axis::Row => assert_close(&[after.w], &[before.w]),
        }
        round_trip_and_settled(&mut layout);
    }
}

#[test]
fn several_merged_holders_keep_only_first_and_use_existing_focus_fallback() {
    let mut right = wrapped(3, 4, 5, "files:e", Size::Weight(1.));
    let Node::Split(split) = &mut right.node else {
        unreachable!()
    };
    let Node::Pane(pane) = &mut split.children[1].node else {
        unreachable!()
    };
    pane.tabs = vec!["remove:other".into()];
    pane.active = pane.tabs.first().cloned();
    let mut layout = layout(
        vec![wrapped(0, 1, 2, "files:b", Size::Weight(1.)), right],
        4,
    );
    assert_eq!(layout.validate(), Ok(()));
    layout.retain(|tab| !tab.starts_with("remove:"));
    assert_eq!(pane_order(&layout), [PaneId(0), PaneId(1), PaneId(3)]);
    assert!(layout.holds_room(PaneId(1)));
    assert_eq!(layout.focused, PaneId(3));
    assert_eq!(fixed_size(&layout, PaneId(0)), Size::Fixed(100.));
    assert_eq!(fixed_size(&layout, PaneId(3)), Size::Fixed(100.));
    round_trip_and_settled(&mut layout);
}

#[test]
fn removed_holder_donates_to_previous_weighted_survivor_before_following_one() {
    let mut layout = layout(
        vec![
            leaf(0, &["files:a", "files:a2"], Size::Weight(2.), false),
            wrapped(1, 2, 3, "files:b", Size::Weight(1.)),
            leaf(4, &["files:z"], Size::Weight(3.), false),
        ],
        0,
    );
    layout.root.pane_mut(PaneId(0)).unwrap().pinned = 1;
    assert_eq!(layout.validate(), Ok(()));
    layout.close("remove:wrapper");
    assert_eq!(pane_order(&layout), [PaneId(0), PaneId(1), PaneId(4)]);
    assert_eq!(
        layout.sizes(&[]),
        Some(vec![Size::Weight(1.), Size::Fixed(100.), Size::Weight(1.),])
    );
    assert_eq!(
        layout.pane(PaneId(0)).unwrap().tabs,
        ["files:a", "files:a2"]
    );
    assert_eq!(layout.pane(PaneId(0)).unwrap().pinned, 1);
    assert_eq!(layout.active(PaneId(0)), Some("files:a"));
    round_trip_and_settled(&mut layout);
}

#[test]
fn a_nonempty_nested_weighted_sibling_can_take_the_room() {
    let mut layout = layout(
        vec![
            split(
                Axis::Row,
                Size::Weight(1.),
                vec![
                    leaf(0, &["files:a"], Size::Weight(1.), false),
                    leaf(4, &["files:z"], Size::Weight(1.), false),
                ],
            ),
            wrapped(1, 2, 3, "files:b", Size::Weight(1.)),
        ],
        4,
    );
    assert_eq!(layout.validate(), Ok(()));
    layout.close("remove:wrapper");
    assert_eq!(pane_order(&layout), [PaneId(0), PaneId(4), PaneId(1)]);
    assert_eq!(layout.focused, PaneId(4));
    assert_eq!(
        layout.sizes(&[]),
        Some(vec![Size::Weight(1.), Size::Fixed(100.)])
    );
    round_trip_and_settled(&mut layout);
}

#[test]
fn fixed_outer_merge_drops_its_now_fixed_empty_holder_without_weight_donation() {
    let mut layout = layout(
        vec![
            leaf(0, &["files:a"], Size::Weight(1.), false),
            wrapped(1, 2, 3, "files:b", Size::Fixed(300.)),
        ],
        1,
    );
    assert_eq!(layout.validate(), Ok(()));
    layout.close("remove:wrapper");
    assert_eq!(pane_order(&layout), [PaneId(0), PaneId(1)]);
    assert_eq!(
        layout.sizes(&[]),
        Some(vec![Size::Weight(1.), Size::Fixed(100.)])
    );
    round_trip_and_settled(&mut layout);
}

#[test]
fn explicit_keep_survives_even_when_fixed_and_empty() {
    for size in [Size::Weight(1.), Size::Fixed(100.)] {
        let mut layout = layout(
            vec![
                leaf(0, &[], size, size.is_fixed()),
                leaf(1, &[], Size::Weight(1.), false),
            ],
            0,
        );
        layout.root.normalize(Some(PaneId(0)));
        assert!(layout.pane(PaneId(0)).is_some());
        assert_eq!(layout.focused, PaneId(0));
        assert_eq!(layout.validate(), Ok(()));
        if size.is_fixed() {
            assert_eq!(pane_order(&layout), [PaneId(0), PaneId(1)]);
            assert!(layout.holds_room(PaneId(1)));
        } else {
            assert_eq!(pane_order(&layout), [PaneId(0)]);
        }
    }
}

#[test]
fn genuine_holder_stays_until_all_tabs_are_gone_then_weighted_focus_is_kept() {
    let mut layout = TabLayout {
        root: holder(0, 1, "files:b").node,
        focused: PaneId(1),
        next_id: 2,
        ..TabLayout::default()
    };
    assert!(layout.holds_room(PaneId(1)));
    round_trip_and_settled(&mut layout);
    assert_eq!(pane_order(&layout), [PaneId(0), PaneId(1)]);
    layout.retain(|_| false);
    assert_eq!(pane_order(&layout), [PaneId(1)]);
    assert_eq!(layout.focused, PaneId(1));
    assert!(!layout.panes()[0].locked);
    round_trip_and_settled(&mut layout);
}

#[test]
fn alternating_axis_wrappers_collapse_in_one_settlement_without_tree_retry() {
    for depth in [1, 3, 9] {
        let mut inner = holder(1, 2, "files:b");
        for level in 0..depth {
            inner = split(
                if level % 2 == 0 {
                    Axis::Row
                } else {
                    Axis::Column
                },
                Size::Weight(1.),
                vec![
                    inner,
                    leaf(
                        3 + level,
                        &[&format!("remove:{level}")],
                        Size::Weight(1.),
                        false,
                    ),
                ],
            );
        }
        let mut layout = layout(
            vec![leaf(0, &["files:a"], Size::Weight(1.), false), inner],
            1,
        );
        assert_eq!(layout.validate(), Ok(()));
        layout.retain(|tab| !tab.starts_with("remove:"));
        assert_eq!(pane_order(&layout), [PaneId(0), PaneId(1)]);
        assert_eq!(layout.focused, PaneId(1));
        assert_eq!(fixed_size(&layout, PaneId(1)), Size::Fixed(100.));
        round_trip_and_settled(&mut layout);
    }
}
