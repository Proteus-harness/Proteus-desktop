//! Restart state of independent windows. Runtime focus stays in each layout;
//! until window placement is persisted, their tabs reopen together.

use super::TabLayout;

impl TabLayout {
    /// Adds tabs from another window without changing this window's focus.
    /// Duplicate ids remain a single tab and pinned tabs stay pinned.
    pub fn include_window(&mut self, other: &Self) {
        let focused = self.focused;
        let active = self.active(focused).map(str::to_owned);
        for pane in other.panes() {
            for id in &pane.tabs {
                if self.contains(id) {
                    continue;
                }
                self.open(id, |_| false);
                self.keep(id);
                self.set_pinned(id, other.is_pinned(id));
            }
        }
        if let Some(active) = active {
            self.activate(focused, &active);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transferring_last_tab_does_not_reopen_it_as_closed() {
        let mut source = TabLayout::default();
        source.open("terminal:1", |_| false);
        source.set_pinned("terminal:1", true);
        let mut target = TabLayout::default();
        target.open("terminal:1", |_| false);
        target.set_pinned("terminal:1", source.is_pinned("terminal:1"));
        source.extract("terminal:1");
        assert!(!source.contains("terminal:1"));
        assert_eq!(source.closed_ids().count(), 0);
        assert!(target.is_pinned("terminal:1"));
        assert_eq!(source.validate(), Ok(()));
        assert_eq!(target.validate(), Ok(()));
    }

    #[test]
    fn independent_windows_keep_focus_and_restore_all_tabs_once() {
        let mut source = TabLayout::default();
        source.open("chat:/first", |_| false);
        source.open("files:one.rs", |_| false);
        let mut target = TabLayout::default();
        target.open("terminal:1", |_| false);
        target.set_pinned("terminal:1", true);
        let target_before = target.clone();
        let mut saved = source.clone();
        saved.include_window(&target);
        saved.include_window(&target);
        assert_eq!(saved.active(saved.focused), Some("files:one.rs"));
        assert!(saved.contains("chat:/first"));
        assert!(saved.is_pinned("terminal:1"));
        assert_eq!(
            saved
                .panes()
                .iter()
                .map(|pane| pane.tabs.len())
                .sum::<usize>(),
            3
        );
        assert_eq!(target, target_before);
        assert_eq!(saved.validate(), Ok(()));
    }
}
