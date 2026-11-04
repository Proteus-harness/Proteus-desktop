//! Rename a document identity without reopening its pane or navigation state.
use super::{ChatSpaces, TabLayout};

impl TabLayout {
    /// Another native window already owns the destination. Remove the source
    /// without a close visit; its reopened/history address follows the rename.
    pub fn retarget_document_to_existing(&mut self, from: &str, to: &str) {
        if from == to || super::is_chat(from) || super::is_chat(to) {
            return;
        }
        self.extract(from);
        self.retarget_closed(from, to);
    }

    /// Keep the existing destination on collision. If the source was the
    /// focused page, its existing destination becomes the focused page.
    pub fn retarget_document(&mut self, from: &str, to: &str) {
        if from == to || super::is_chat(from) || super::is_chat(to) {
            return;
        }
        let focused_source = self.active(self.focused) == Some(from);
        if self.contains(from) && self.contains(to) {
            self.extract(from);
            if focused_source && let Some(pane) = self.pane_of(to) {
                self.activate(pane, to);
            }
        } else {
            let panes = self.pane_ids();
            for id in panes {
                let pane = self.root.pane_mut(id).unwrap();
                for tab in pane.tabs.iter_mut().chain(pane.history.iter_mut()) {
                    replace(tab, from, to);
                }
                if let Some(active) = &mut pane.active {
                    replace(active, from, to);
                }
            }
            if let Some(preview) = &mut self.preview {
                replace(preview, from, to);
            }
        }
        self.retarget_closed(from, to);
    }

    fn retarget_closed(&mut self, from: &str, to: &str) {
        for closed in &mut self.closed {
            replace(&mut closed.id, from, to);
        }
        let mut seen = std::collections::HashSet::new();
        self.closed.reverse();
        self.closed.retain(|tab| seen.insert(tab.id.clone()));
        self.closed.reverse();
    }
}

impl ChatSpaces {
    pub fn retarget_document_to_existing(&mut self, root: &str, from: &str, to: &str) -> bool {
        let Some(layout) = self.roots.get_mut(root) else {
            return false;
        };
        layout.retarget_document_to_existing(from, to);
        true
    }

    /// Exact retained session area only; another session with identical
    /// document arguments is independent.
    pub fn retarget_document(&mut self, root: &str, from: &str, to: &str) -> bool {
        let Some(layout) = self.roots.get_mut(root) else {
            return false;
        };
        layout.retarget_document(from, to);
        true
    }
}

fn replace(value: &mut String, from: &str, to: &str) {
    if value == from {
        *value = to.to_owned();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Edge, PaneId};

    #[test]
    fn document_retarget_preserves_panes_preview_pins_focus_and_closed_history() {
        let mut layout = TabLayout::default();
        layout.open("chat:/session", |_| false);
        layout.open("fixture:old", |_| false);
        layout.split("fixture:old", PaneId(0), Edge::Right);
        let pane = layout.pane_of("fixture:old").unwrap();
        layout.preview = Some("fixture:old".into());
        let before = layout.clone();
        layout.retarget_document("fixture:old", "fixture:new");
        assert_eq!(layout.pane_of("fixture:new"), Some(pane));
        assert_eq!(layout.focused, before.focused);
        assert_eq!(layout.preview.as_deref(), Some("fixture:new"));
        let restored: TabLayout =
            serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!(restored.pane_of("fixture:new"), Some(pane));
        assert_eq!(restored.preview.as_deref(), Some("fixture:new"));
        assert_eq!(restored.focused, layout.focused);
        layout.keep("fixture:new");
        layout.set_pinned("fixture:new", true);
        layout.retarget_document("fixture:new", "fixture:kept");
        assert!(layout.is_pinned("fixture:kept"));
        assert!(layout.preview.is_none());
        layout.close("fixture:kept");
        layout.retarget_document("fixture:kept", "fixture:reopen");
        assert_eq!(layout.closed_ids().collect::<Vec<_>>(), ["fixture:reopen"]);
        assert!(layout.validate().is_ok());
        let saved = serde_json::to_string(&layout).unwrap();
        let restored: TabLayout = serde_json::from_str(&saved).unwrap();
        assert!(restored.validate().is_ok());
    }

    #[test]
    fn existing_destination_is_reused_and_other_session_is_untouched() {
        let mut spaces = ChatSpaces::default();
        spaces.select("chat:/one");
        let mut layout = spaces.active_layout().clone();
        layout.open("fixture:new", |_| false);
        layout.set_pinned("fixture:new", true);
        layout.open("fixture:old", |_| false);
        layout.split("fixture:old", PaneId(0), Edge::Right);
        let destination = layout.pane_of("fixture:new").unwrap();
        layout.preview = Some("fixture:old".into());
        spaces.save_active(layout);
        spaces.select("chat:/two");
        let mut other = spaces.active_layout().clone();
        other.open("fixture:old", |_| false);
        spaces.save_active(other.clone());
        assert!(spaces.retarget_document("chat:/one", "fixture:old", "fixture:new"));
        assert_eq!(spaces.active_layout(), &other);
        spaces.select("chat:/one");
        let layout = spaces.active_layout();
        assert!(!layout.contains("fixture:old"));
        assert_eq!(layout.pane_of("fixture:new"), Some(destination));
        assert!(layout.is_pinned("fixture:new"));
        assert!(layout.preview.is_none());
        assert_eq!(layout.active(layout.focused), Some("fixture:new"));
        assert!(spaces.validate().is_ok());
    }
}
