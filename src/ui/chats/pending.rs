//! Loading fills an existing selection; network completion never navigates.
use std::{cell::Cell, collections::HashMap, rc::Rc};

use crate::tabs::{ChatSpaces, TabLayout};

/// Consume only this load's acknowledgement. An old request of the same
/// thread cannot consume a newer request or restore a tab the user closed.
pub(super) fn take<T: Copy>(
    opening: &mut HashMap<String, Rc<Cell<T>>>,
    id: &str,
    pending: &Rc<Cell<T>>,
    spaces: &ChatSpaces,
) -> Option<T> {
    if !opening
        .get(id)
        .is_some_and(|active| Rc::ptr_eq(active, pending))
    {
        return None;
    }
    opening.remove(id);
    (spaces.contains(id) || spaces.roots().any(|root| root == id)).then(|| pending.get())
}

pub(super) fn may_focus(
    layout: &TabLayout,
    id: &str,
    settings_open: bool,
    requested: bool,
) -> bool {
    requested && !settings_open && layout.active(layout.focused) == Some(id)
}

pub(super) fn take_creation(current: &mut Option<Rc<()>>, ticket: &Rc<()>) -> bool {
    if current
        .as_ref()
        .is_some_and(|active| Rc::ptr_eq(active, ticket))
    {
        current.take();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_in_reverse_order_do_not_change_the_latest_selection() {
        let mut layout = TabLayout::default();
        let first = Rc::new(Cell::new(true));
        let second = Rc::new(Cell::new(true));
        let mut opening = HashMap::from([
            ("codex:a".into(), first.clone()),
            ("codex:b".into(), second.clone()),
        ]);
        layout.open("codex:a", |_| false);
        layout.open("codex:b", |_| false);
        assert_eq!(
            take(
                &mut opening,
                "codex:b",
                &second,
                &ChatSpaces::from_layout(layout.clone())
            ),
            Some(true)
        );
        assert!(may_focus(&layout, "codex:b", false, true));
        assert_eq!(
            take(
                &mut opening,
                "codex:a",
                &first,
                &ChatSpaces::from_layout(layout.clone())
            ),
            Some(true)
        );
        assert!(!may_focus(&layout, "codex:a", false, true));
        assert_eq!(layout.active(layout.focused), Some("codex:b"));
    }

    #[test]
    fn replaced_preview_and_closed_chat_ignore_delayed_replies() {
        for close in [false, true] {
            let mut layout = TabLayout::default();
            let pending = Rc::new(Cell::new(true));
            let mut opening = HashMap::from([("codex:a".into(), pending.clone())]);
            layout.open_preview("codex:a", |_| false);
            if close {
                layout.close("codex:a");
            } else {
                layout.open_preview("codex:b", |_| false);
            }
            assert_eq!(
                take(
                    &mut opening,
                    "codex:a",
                    &pending,
                    &ChatSpaces::from_layout(layout.clone())
                ),
                None
            );
            assert!(opening.is_empty());
            assert!(!layout.contains("codex:a"));
        }
    }

    #[test]
    fn old_reply_cannot_consume_reopened_chat_request() {
        let mut layout = TabLayout::default();
        layout.open("codex:a", |_| false);
        let old = Rc::new(Cell::new(true));
        let replacement = Rc::new(Cell::new(false));
        let mut opening = HashMap::from([("codex:a".into(), replacement.clone())]);
        assert_eq!(
            take(
                &mut opening,
                "codex:a",
                &old,
                &ChatSpaces::from_layout(layout.clone())
            ),
            None
        );
        assert_eq!(
            take(
                &mut opening,
                "codex:a",
                &replacement,
                &ChatSpaces::from_layout(layout.clone())
            ),
            Some(false)
        );
    }

    #[test]
    fn background_area_reply_fills_original_tab_without_focus() {
        let mut spaces = ChatSpaces::default();
        spaces.select("codex:a");
        spaces.select("codex:b");
        let pending = Rc::new(Cell::new(true));
        let mut opening = HashMap::from([("codex:a".into(), pending.clone())]);
        assert_eq!(take(&mut opening, "codex:a", &pending, &spaces), Some(true));
        assert!(!may_focus(spaces.active_layout(), "codex:a", false, true));
        assert_eq!(spaces.active_root(), Some("codex:b"));
    }

    #[test]
    fn opening_settings_or_previewing_preserves_keyboard_focus() {
        let mut layout = TabLayout::default();
        layout.open("codex:a", |_| false);
        assert!(!may_focus(&layout, "codex:a", true, true));
        assert!(!may_focus(&layout, "codex:a", false, false));
        assert!(may_focus(&layout, "codex:a", false, true));
    }

    #[test]
    fn only_latest_new_chat_may_select_even_when_layout_did_not_change() {
        let first = Rc::new(());
        let latest = Rc::new(());
        let mut current = Some(latest.clone());
        assert!(!take_creation(&mut current, &first));
        assert!(take_creation(&mut current, &latest));
        assert!(!take_creation(&mut current, &first));
        assert!(!take_creation(&mut current, &latest));
    }
}
