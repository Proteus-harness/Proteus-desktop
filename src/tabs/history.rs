// Адаптировано из Zed: crates/workspace/src/pane.rs (closed navigation stack)
//! Reopening closed tabs is shared by chats and every extension document.
//! Moving, preview replacement and invalidating unavailable pages do not
//! count as closing, and never fill the reopen stack.

use super::{PaneId, TabLayout};

const MAX_CLOSED: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct ClosedTab {
    pub id: String,
    pane: PaneId,
    index: usize,
    pinned: bool,
}

impl TabLayout {
    /// The keyboard close command protects pins. Like Zed it first moves
    /// to an unpinned tab, including another pane, rather than closing a pin.
    /// An explicit close button/context action still uses `close` directly.
    pub fn close_active(&mut self) {
        let Some(active) = self.active(self.focused).map(str::to_string) else {
            return;
        };
        if !self.is_pinned(&active) {
            self.close(&active);
            return;
        }
        let mut panes = self.panes();
        panes.sort_by_key(|pane| pane.id != self.focused);
        let next = panes
            .into_iter()
            .find_map(|pane| pane.tabs.get(pane.pinned).map(|tab| (pane.id, tab.clone())));
        if let Some((pane, id)) = next {
            self.activate(pane, &id);
        }
    }

    /// Most recent closed pages, used to check availability before a UI action.
    pub fn closed_ids(&self) -> impl Iterator<Item = &str> {
        self.closed.iter().map(|tab| tab.id.as_str())
    }

    pub(super) fn remember_closed(&mut self, id: &str) {
        if self.preview.as_deref() == Some(id) {
            return;
        }
        let Some(pane) = self.pane_of(id).and_then(|pane| self.pane(pane)) else {
            return;
        };
        let index = pane.position(id).expect("tab is open");
        let closed = ClosedTab {
            id: id.to_string(),
            pane: pane.id,
            index,
            pinned: index < pane.pinned,
        };
        self.closed.retain(|tab| tab.id != id);
        if self.closed.len() == MAX_CLOSED {
            self.closed.remove(0);
        }
        self.closed.push(closed);
    }

    /// Takes the latest closed page that is still available. A tab already
    /// opened by another action is skipped rather than opened twice.
    pub fn take_closed(&mut self, available: impl Fn(&str) -> bool) -> Option<ClosedTab> {
        while let Some(tab) = self.closed.pop() {
            if !self.contains(&tab.id) && available(&tab.id) {
                return Some(tab);
            }
        }
        None
    }

    /// Restores the page after its document or session is ready. The old
    /// position and pin survive while the pane does; a removed pane falls
    /// back to the focused one without recreating an obsolete split.
    pub fn restore_closed(&mut self, tab: &ClosedTab) {
        if let Some(pane) = self.pane_of(&tab.id) {
            self.activate(pane, &tab.id);
            self.keep(&tab.id);
            return;
        }
        let original = self.pane(tab.pane);
        let before = original.and_then(|pane| pane.tabs.get(tab.index)).cloned();
        let pane = original.map_or(self.focused, |pane| pane.id);
        self.put(&tab.id, pane, before.as_deref(), tab.pinned);
    }
}
