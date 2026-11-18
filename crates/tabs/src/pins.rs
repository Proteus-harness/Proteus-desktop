//! Pinned tabs: kept in front of their pane and left alone by closing the
//! others.

use super::layout::TabLayout;

impl TabLayout {
    /// Pins or unpins a tab; a pinned preview becomes a regular tab.
    pub fn set_pinned(&mut self, id: &str, pinned: bool) {
        let Some(pane) = self.pane_of(id) else {
            return;
        };
        if pinned {
            self.keep(id);
        }
        self.root
            .pane_mut(pane)
            .expect("pane exists")
            .set_pinned(id, pinned);
    }

    /// Unpinned tabs of the pane of `id` besides `id` itself.
    pub fn others(&self, id: &str) -> Vec<String> {
        let (left, right) = self.unpinned_neighbors(id);
        left.iter().chain(right).cloned().collect()
    }

    /// Unpinned tabs to the right of `id` in its pane.
    pub fn right_of(&self, id: &str) -> Vec<String> {
        self.unpinned_neighbors(id).1.to_vec()
    }

    /// Whether "close others" and "close to the right" would each close a tab.
    pub fn close_availability(&self, id: &str) -> (bool, bool) {
        let (left, right) = self.unpinned_neighbors(id);
        (!left.is_empty() || !right.is_empty(), !right.is_empty())
    }

    /// Eligible tabs on each side of `id`, in the owning pane's strip order.
    fn unpinned_neighbors(&self, id: &str) -> (&[String], &[String]) {
        for pane in self.panes() {
            if let Some(at) = pane.position(id) {
                let left = pane.tabs.get(pane.pinned..at).unwrap_or_default();
                let right = pane.tabs.get(pane.pinned.max(at + 1)..).unwrap_or_default();
                return (left, right);
            }
        }
        (&[], &[])
    }

    /// Closes the unpinned tabs of the pane of `id` besides `id`, which
    /// becomes active.
    pub fn close_others(&mut self, id: &str) {
        self.close_all(self.others(id), id);
    }

    /// Closes the unpinned tabs to the right of `id`, which becomes active.
    pub fn close_right(&mut self, id: &str) {
        self.close_all(self.right_of(id), id);
    }

    fn close_all(&mut self, tabs: Vec<String>, keep: &str) {
        let Some(pane) = self.pane_of(keep) else {
            return;
        };
        self.activate(pane, keep);
        let ids = self.pane_ids();
        for tab in tabs {
            self.remember_closed(&tab);
            self.detach(&tab);
        }
        self.settle(&ids);
    }
}
