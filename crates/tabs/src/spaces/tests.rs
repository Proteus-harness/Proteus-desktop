use super::*;
use crate::Edge;

const FIRST: &str = "chat:/first";
const SECOND: &str = "codex:second";
const CHILD: &str = "chat:/first/child";

fn populated() -> ChatSpaces {
    let mut spaces = ChatSpaces::default();
    assert!(spaces.select(FIRST));
    let mut layout = spaces.active_layout().clone();
    layout.open(CHILD, |_| false);
    layout.open("files:first.rs", |_| true);
    layout.keep("files:first.rs");
    layout.set_pinned("files:first.rs", true);
    spaces.save_active(layout);
    spaces
}

#[test]
fn roots_keep_independent_layouts_focus_and_child_command_context() {
    let mut spaces = populated();
    let first = spaces.active_layout().clone();
    assert_eq!(first.active_chat(None).as_deref(), Some(CHILD));
    assert_eq!(spaces.active_root(), Some(FIRST));
    assert!(spaces.select(SECOND));
    assert_eq!(spaces.active_layout().chats().collect::<Vec<_>>(), [SECOND]);
    let mut second = spaces.active_layout().clone();
    second.open("terminal:second", |_| true);
    second.split_active(Edge::Bottom);
    spaces.save_active(second.clone());
    spaces.select(FIRST);
    assert_eq!(spaces.active_layout(), &first);
    assert!(spaces.active_layout().is_pinned("files:first.rs"));
    spaces.select(SECOND);
    assert_eq!(spaces.active_layout(), &second);
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn documents_with_the_same_id_can_exist_in_distinct_chat_areas() {
    let mut spaces = populated();
    spaces.select(SECOND);
    let mut layout = spaces.active_layout().clone();
    layout.open("files:first.rs", |_| false);
    spaces.save_active(layout);
    assert_eq!(
        spaces
            .all_tabs()
            .filter(|tab| *tab == "files:first.rs")
            .count(),
        2
    );
    assert_eq!(spaces.owner_of_chat(CHILD), Some(FIRST));
    assert_eq!(spaces.owner_of_chat("files:first.rs"), None);
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn detached_fragment_retains_ownership_without_reopening_root() {
    let mut spaces = ChatSpaces::default();
    let mut fragment = TabLayout::default();
    fragment.open("terminal:detached", |_| false);
    assert!(spaces.set_active(Some(FIRST), fragment.clone()));
    assert!(!spaces.contains(FIRST));
    assert_eq!(spaces.owner_of_chat(FIRST), Some(FIRST));
    assert!(!spaces.active_layout().contains(FIRST));
    spaces.select(FIRST);
    assert_eq!(spaces.active_layout(), &fragment);
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn deletion_extracts_children_and_closed_history_from_every_area() {
    let mut spaces = populated();
    let mut first = spaces.active_layout().clone();
    first.close(CHILD);
    assert_eq!(first.closed_ids().collect::<Vec<_>>(), [CHILD]);
    spaces.save_active(first);
    spaces.select(SECOND);
    let mut second = spaces.active_layout().clone();
    second.open(CHILD, |_| false);
    spaces.save_active(second);
    spaces.remove_chats([CHILD]);
    assert!(!spaces.contains(CHILD));
    assert!(
        spaces
            .all_layouts()
            .all(|layout| !layout.closed_ids().any(|id| id == CHILD))
    );
    assert!(spaces.contains(FIRST));
    assert_eq!(spaces.active_root(), Some(SECOND));
    spaces.remove_chats([SECOND]);
    assert_eq!(spaces.active_root(), None);
    assert_eq!(spaces.active_layout(), &TabLayout::default());
    assert!(spaces.contains(FIRST));
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn deleting_root_drops_its_area_without_touching_other_layouts() {
    let mut spaces = populated();
    spaces.select(SECOND);
    let second = spaces.active_layout().clone();
    spaces.remove_chats([FIRST, "files:first.rs"]);
    assert!(!spaces.contains(FIRST));
    assert!(!spaces.contains("files:first.rs"));
    assert!(!spaces.contains(CHILD));
    assert_eq!(spaces.active_layout(), &second);
    assert_eq!(spaces.active_root(), Some(SECOND));
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn merge_windows_combines_matching_roots_and_keeps_other_trees_separate() {
    let mut spaces = populated();
    let before = spaces.active_layout().clone();
    let mut other = ChatSpaces::default();
    let mut fragment = TabLayout::default();
    fragment.open("terminal:detached", |_| false);
    fragment.set_pinned("terminal:detached", true);
    other.set_active(Some(FIRST), fragment);
    other.select(SECOND);
    let other_second = other.active_layout().clone();
    spaces.include_window(&other);
    spaces.include_window(&other);
    assert_eq!(spaces.active_root(), Some(FIRST));
    assert_eq!(spaces.active_layout().focused, before.focused);
    assert_eq!(
        spaces.active_layout().active(before.focused),
        before.active(before.focused)
    );
    assert!(spaces.active_layout().is_pinned("terminal:detached"));
    assert_eq!(
        spaces
            .all_tabs()
            .filter(|tab| *tab == "terminal:detached")
            .count(),
        1
    );
    assert!(!spaces.active_layout().contains(SECOND));
    spaces.select(SECOND);
    assert_eq!(spaces.active_layout(), &other_second);
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn loose_documents_stay_separate_and_roundtrip_preserves_all_roots() {
    let mut spaces = ChatSpaces::default();
    let mut loose = TabLayout::default();
    loose.open("files:unowned", |_| false);
    spaces.save_active(loose.clone());
    spaces.select(FIRST);
    spaces.select(SECOND);
    assert!(!spaces.active_layout().contains("files:unowned"));
    let restored: ChatSpaces =
        serde_json::from_str(&serde_json::to_string(&spaces).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&spaces).unwrap()
    );
    assert_eq!(restored.all_layouts().count(), 3);
    assert_eq!(restored.all_tabs().count(), 3);
    assert_eq!(restored.validate(), Ok(()));
    spaces.set_active(None, loose.clone());
    assert_eq!(spaces.active_root(), None);
    assert_eq!(spaces.active_layout(), &loose);
}

#[test]
fn malformed_identity_and_dangling_active_are_rejected() {
    let mut spaces = populated();
    let before = spaces.clone();
    for bad in ["files:main.rs", "chat:", "codex:", ""] {
        assert!(!spaces.select(bad));
        assert!(!spaces.set_active(Some(bad), TabLayout::default()));
        assert_eq!(spaces, before);
    }
    let mut encoded = serde_json::to_value(&spaces).unwrap();
    encoded["active"] = "chat:/absent".into();
    let invalid: ChatSpaces = serde_json::from_value(encoded).unwrap();
    assert!(invalid.validate().unwrap_err().contains("has no chat area"));
    let invalid: ChatSpaces = serde_json::from_value(serde_json::json!({
        "roots": { "files:invalid": TabLayout::default() }
    }))
    .unwrap();
    assert!(
        invalid
            .validate()
            .unwrap_err()
            .contains("invalid chat area root")
    );
}

#[test]
fn legacy_layout_migration_preserves_tree_and_last_command_chat() {
    let legacy = populated().active_layout().clone();
    let spaces = ChatSpaces::from_layout(legacy.clone());
    assert_eq!(spaces.active_root(), Some(CHILD));
    assert_eq!(spaces.active_layout(), &legacy);
    assert!(spaces.contains(FIRST));
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn failed_resume_in_hidden_area_extracts_chat_but_keeps_area_documents() {
    let mut spaces = populated();
    let mut first = spaces.active_layout().clone();
    first.close(FIRST);
    first.open(FIRST, |_| false);
    spaces.save_active(first);
    spaces.select(SECOND);
    let active_before = spaces.active_layout().clone();
    spaces.extract_chat(FIRST);
    assert!(!spaces.contains(FIRST));
    assert_eq!(spaces.owner_of_chat(FIRST), Some(FIRST));
    assert_eq!(spaces.active_root(), Some(SECOND));
    assert_eq!(spaces.active_layout(), &active_before);
    spaces.select(FIRST);
    assert!(!spaces.active_layout().contains(FIRST));
    assert!(spaces.active_layout().contains(CHILD));
    assert!(spaces.active_layout().is_pinned("files:first.rs"));
    assert!(!spaces.active_layout().closed_ids().any(|id| id == FIRST));
    assert_eq!(spaces.validate(), Ok(()));
}

#[test]
fn selecting_legacy_chat_can_move_its_tab_to_own_area_without_duplicates() {
    let mut legacy = TabLayout::default();
    legacy.open(FIRST, |_| false);
    legacy.open(SECOND, |_| false);
    legacy.open("files:legacy", |_| true);
    let mut spaces = ChatSpaces::default();
    spaces.set_active(Some(FIRST), legacy);
    spaces.select(SECOND);
    let destination = spaces.active_layout().clone();
    spaces.extract_chat_except(SECOND, SECOND);
    assert_eq!(spaces.active_layout(), &destination);
    assert_eq!(spaces.owner_of_chat(SECOND), Some(SECOND));
    assert_eq!(spaces.all_tabs().filter(|id| *id == SECOND).count(), 1);
    spaces.select(FIRST);
    assert!(!spaces.active_layout().contains(SECOND));
    assert!(spaces.active_layout().contains(FIRST));
    assert!(spaces.active_layout().contains("files:legacy"));
    assert_eq!(spaces.validate(), Ok(()));
}
