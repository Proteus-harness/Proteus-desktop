use super::super::{Edge, PaneId, TabLayout};
use super::{blank, shape};

fn five() -> TabLayout {
    let mut layout = blank();
    for tab in ["a", "b", "c", "d", "e"] {
        layout.open(tab, |_| false);
    }
    layout
}

#[test]
fn closing_walks_the_whole_recently_visited_history() {
    let mut layout = five();
    for tab in ["c", "a", "e", "b"] {
        layout.activate(PaneId(0), tab);
    }
    for (closed, expected) in [("b", "e"), ("e", "a"), ("a", "c"), ("c", "d")] {
        layout.close(closed);
        assert_eq!(layout.active(PaneId(0)), Some(expected));
        assert_eq!(layout.validate(), Ok(()));
    }
}

#[test]
fn preview_replacement_does_not_leave_a_closed_history_entry() {
    let mut layout = five();
    layout.activate(PaneId(0), "b");
    layout.open_preview("preview-one", |_| false);
    layout.open_preview("preview-two", |_| false);
    layout.close("preview-two");
    assert_eq!(layout.active(PaneId(0)), Some("b"));
    assert!(layout.take_closed(|_| true).is_none());
}

#[test]
fn reopen_restores_the_original_position_and_pin() {
    let mut layout = five();
    layout.set_pinned("b", true);
    layout.close("b");
    let closed = layout.take_closed(|_| true).unwrap();
    layout.restore_closed(&closed);
    assert_eq!(shape(&layout), "0[b^*, a, c, d, e]");
    layout.close("c");
    let closed = layout.take_closed(|_| true).unwrap();
    layout.restore_closed(&closed);
    assert_eq!(shape(&layout), "0[b^, a, c*, d, e]");
}

#[test]
fn reopen_skips_unavailable_and_already_open_pages_and_removed_panes() {
    let mut layout = five();
    layout.split("e", PaneId(0), Edge::Right);
    layout.close("e");
    layout.close("b");
    layout.close("c");
    layout.open("b", |_| false);
    let closed = layout.take_closed(|id| id != "c").unwrap();
    assert_eq!(closed.id, "e");
    layout.restore_closed(&closed);
    assert_eq!(shape(&layout), "0[a, d, b, e*]");
    assert!(layout.take_closed(|_| true).is_none());
}

#[test]
fn moving_and_invalidating_pages_never_fill_the_closed_stack() {
    let mut layout = five();
    layout.split("e", PaneId(0), Edge::Bottom);
    layout.move_tab("e", PaneId(0), None);
    layout.retain(|id| id != "b");
    assert!(layout.take_closed(|_| true).is_none());
}

#[test]
fn keyboard_close_protects_pins_and_moves_to_an_unpinned_page() {
    let mut layout = five();
    layout.set_pinned("a", true);
    layout.activate(PaneId(0), "a");
    layout.close_active();
    assert_eq!(shape(&layout), "0[a^, b*, c, d, e]");
    assert!(layout.take_closed(|_| true).is_none());
    layout.close_active();
    assert!(!layout.contains("b"));
    assert_eq!(layout.active(PaneId(0)), Some("a"));
    let closed = layout.take_closed(|_| true).unwrap();
    assert_eq!(closed.id, "b");
    layout.close_others("a");
    layout.close_active();
    assert_eq!(shape(&layout), "0[a^*]");
}
