use super::{A, B, PaneId, TabLayout, blank};
use crate::Edge;

const FILE: &str = "files:src/main.rs";

fn hidden_last_chat() -> TabLayout {
    let mut layout = blank();
    layout.open(A, |_| false);
    layout.open(B, |_| false);
    layout.open(FILE, |_| false);
    layout
}

fn restored(layout: &TabLayout) -> TabLayout {
    let json = serde_json::to_string(layout).unwrap();
    let back: TabLayout = serde_json::from_str(&json).unwrap();
    assert_eq!(back.validate(), Ok(()));
    back
}

#[test]
fn document_focus_restores_the_last_chat_instead_of_the_first_tab() {
    let layout = restored(&hidden_last_chat());
    assert_eq!(layout.active(layout.focused), Some(FILE));
    assert_eq!(layout.active_chat(None).as_deref(), Some(B));
    // A stale UI value cannot overwrite a newer explicit chat selection.
    assert_eq!(layout.active_chat(Some(A)).as_deref(), Some(B));
    assert_eq!(layout.chats().collect::<Vec<_>>(), [A, B]);
}

#[test]
fn split_focus_and_tab_movement_update_context_without_reordering_tabs() {
    let mut layout = hidden_last_chat();
    layout.split(A, PaneId(0), Edge::Right);
    let side = layout.focused;
    layout.focus(PaneId(0));
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(A));
    layout.activate(PaneId(0), B);
    layout.activate(PaneId(0), FILE);
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(B));
    layout.focus_neighbor(Edge::Right);
    layout.focus_neighbor(Edge::Left);
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(A));
    layout.move_tab(B, side, None);
    layout.focus(PaneId(0));
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(B));
    layout.close_pane(side);
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(B));
}

#[test]
fn closed_and_unavailable_chats_leave_no_stale_command_target() {
    let mut layout = hidden_last_chat();
    layout.close(A);
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(B));
    layout.close(B);
    assert_eq!(restored(&layout).active_chat(None), None);
    assert_eq!(layout.active_chat(Some(FILE)), None);

    let mut layout = hidden_last_chat();
    layout.retain(|id| id != B);
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(A));
    layout.retain(|id| id == FILE);
    assert_eq!(restored(&layout).active_chat(None), None);
}

#[test]
fn detaching_a_chat_gives_each_window_its_own_valid_context() {
    let mut source = hidden_last_chat();
    let mut target = blank();
    target.open(B, |_| false);
    source.extract(B);
    assert_eq!(restored(&source).active_chat(None).as_deref(), Some(A));
    assert_eq!(restored(&target).active_chat(None).as_deref(), Some(B));
    assert_eq!(source.closed_ids().count(), 0);
}

#[test]
fn merging_windows_preserves_the_receiving_windows_command_context() {
    let mut saved = hidden_last_chat();
    let mut other = blank();
    other.open("chat:/s/c", |_| false);
    other.open("chat:/s/d", |_| false);
    other.open("terminal:1", |_| false);
    saved.include_window(&other);
    assert_eq!(restored(&saved).active_chat(None).as_deref(), Some(B));
    assert_eq!(saved.active(saved.focused), Some(FILE));
    saved.close(FILE);
    // Importing background windows must not change the pane's tab MRU.
    assert_eq!(saved.active(saved.focused), Some(B));

    let mut empty = blank();
    empty.include_window(&other);
    assert_eq!(
        restored(&empty).active_chat(None).as_deref(),
        Some("chat:/s/d")
    );

    other.close("terminal:1");
    other.activate(other.focused, "chat:/s/c");
    let mut empty = blank();
    empty.include_window(&other);
    assert_eq!(
        restored(&empty).active_chat(None).as_deref(),
        Some("chat:/s/c")
    );
}

#[test]
fn replacing_a_preview_chat_with_a_document_invalidates_its_context() {
    let mut layout = blank();
    layout.open(A, |_| false);
    layout.open_preview(B, |_| false);
    layout.open_preview(FILE, |_| false);
    assert!(!layout.contains(B));
    assert_eq!(restored(&layout).active_chat(None).as_deref(), Some(A));
}

#[test]
fn validation_rejects_a_missing_or_non_chat_command_target() {
    let mut layout = hidden_last_chat();
    layout.last_chat = Some("chat:/s/gone".into());
    assert!(layout.validate().is_err());
    layout.last_chat = Some(FILE.into());
    assert!(layout.validate().is_err());
}
