//! Independent working areas of root conversations within a native window.
//! A root owns its pane tree; a focused child chat never changes that owner.

use std::collections::{BTreeMap, HashSet};

use serde::{Deserialize, Serialize};

use super::{TabLayout, is_chat};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChatSpaces {
    active: Option<String>,
    pub(super) roots: BTreeMap<String, TabLayout>,
    loose: TabLayout,
}

impl ChatSpaces {
    /// Saved-file migration: retain the entire former pane tree, including
    /// other conversations and documents, under its last command chat.
    pub fn from_layout(layout: TabLayout) -> Self {
        let mut spaces = Self::default();
        let root = layout.active_chat(None).filter(|root| valid_root(root));
        spaces.set_active(root.as_deref(), layout);
        spaces
    }

    pub fn active_root(&self) -> Option<&str> {
        self.active.as_deref()
    }

    pub fn active_layout(&self) -> &TabLayout {
        self.active
            .as_ref()
            .and_then(|root| self.roots.get(root))
            .unwrap_or(&self.loose)
    }

    /// The UI owns the live layout separately and checkpoints it before a
    /// switch or persistence. Saving an empty detached fragment is valid.
    pub fn save_active(&mut self, layout: TabLayout) {
        match &self.active {
            Some(root) => {
                self.roots.insert(root.clone(), layout);
            }
            None => self.loose = layout,
        }
    }

    /// Enter a root's area. Only a newly created area opens the root tab;
    /// revisiting an area preserves its pane focus and document preview.
    pub fn select(&mut self, root: &str) -> bool {
        if !valid_root(root) {
            return false;
        }
        self.roots.entry(root.to_owned()).or_insert_with(|| {
            let mut layout = TabLayout::default();
            layout.open(root, |_| false);
            layout
        });
        self.active = Some(root.to_owned());
        true
    }

    /// Install a transferred fragment without reopening the root chat or
    /// creating a second live conversation stream in the destination window.
    pub fn set_active(&mut self, root: Option<&str>, layout: TabLayout) -> bool {
        if root.is_some_and(|id| !valid_root(id)) {
            return false;
        }
        self.active = root.map(str::to_owned);
        self.save_active(layout);
        true
    }

    /// Whether a tab is actually open, independent of a detached root identity.
    pub fn contains(&self, id: &str) -> bool {
        self.all_layouts().any(|layout| layout.contains(id))
    }

    pub fn roots(&self) -> impl Iterator<Item = &str> {
        self.roots.keys().map(String::as_str)
    }

    pub fn root_layouts(&self) -> impl Iterator<Item = (&str, &TabLayout)> {
        self.roots
            .iter()
            .map(|(root, layout)| (root.as_str(), layout))
    }

    /// Whether there are no actual tabs, including the unowned area.
    pub fn is_empty(&self) -> bool {
        self.all_tabs().next().is_none()
    }

    /// Root identity takes precedence over a migrated copy in another area.
    pub fn owner_of_chat(&self, id: &str) -> Option<&str> {
        if !is_chat(id) {
            return None;
        }
        if let Some((root, _)) = self.roots.get_key_value(id) {
            return Some(root);
        }
        self.roots
            .iter()
            .find(|(_, layout)| layout.contains(id))
            .map(|(root, _)| root.as_str())
    }

    /// Deleting a root deletes its workspace; deleting a child extracts only
    /// that chat from every area. Recently closed entries cannot resurrect it.
    pub fn remove_chats<'a>(&mut self, ids: impl IntoIterator<Item = &'a str>) {
        let ids: HashSet<_> = ids.into_iter().filter(|id| is_chat(id)).collect();
        self.roots.retain(|root, _| !ids.contains(root.as_str()));
        if self
            .active
            .as_deref()
            .is_some_and(|root| ids.contains(root))
        {
            self.active = None;
        }
        for layout in std::iter::once(&mut self.loose).chain(self.roots.values_mut()) {
            for id in &ids {
                layout.extract(id);
            }
            layout.closed.retain(|tab| !ids.contains(tab.id.as_str()));
        }
    }

    /// A failed resume invalidates the chat tab, not its area's documents or
    /// ownership. This also works when the affected area is not selected.
    pub fn extract_chat(&mut self, id: &str) {
        if !is_chat(id) {
            return;
        }
        for layout in std::iter::once(&mut self.loose).chain(self.roots.values_mut()) {
            extract_chat(layout, id);
        }
    }

    /// Move a migrated chat into its explicit area without leaving its old
    /// tab in another root. The destination's pane state is left untouched.
    pub fn extract_chat_except(&mut self, id: &str, root: &str) {
        if !is_chat(id) {
            return;
        }
        extract_chat(&mut self.loose, id);
        for (owner, layout) in &mut self.roots {
            if owner != root {
                extract_chat(layout, id);
            }
        }
    }

    /// Merge corresponding areas across native windows, keeping this window's
    /// focus and active root. Distinct roots retain independent pane trees.
    pub fn include_window(&mut self, other: &Self) {
        self.loose.include_window(&other.loose);
        for (root, layout) in &other.roots {
            match self.roots.get_mut(root) {
                Some(existing) => existing.include_window(layout),
                None => {
                    self.roots.insert(root.clone(), layout.clone());
                }
            }
        }
    }

    pub fn all_layouts(&self) -> impl Iterator<Item = &TabLayout> {
        std::iter::once(&self.loose).chain(self.roots.values())
    }

    /// Actual tabs, excluding root identities whose tabs were detached.
    pub fn all_tabs(&self) -> impl Iterator<Item = &str> {
        self.all_layouts().flat_map(|layout| {
            layout
                .panes()
                .into_iter()
                .flat_map(|pane| pane.tabs.iter().map(String::as_str))
        })
    }

    /// Remove invalid documents from every retained area, including its closed
    /// tab history. Root ownership is unaffected by document cleanup.
    pub fn retain_documents(&mut self, keep: impl Fn(&str) -> bool) {
        for layout in std::iter::once(&mut self.loose).chain(self.roots.values_mut()) {
            layout.retain(|id| is_chat(id) || keep(id));
            layout
                .closed
                .retain(|tab| is_chat(&tab.id) || keep(&tab.id));
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(root) = &self.active
            && !self.roots.contains_key(root)
        {
            return Err(format!("active root `{root}` has no chat area"));
        }
        self.loose.validate()?;
        for (root, layout) in &self.roots {
            if !valid_root(root) {
                return Err(format!("invalid chat area root `{root}`"));
            }
            layout
                .validate()
                .map_err(|error| format!("chat area `{root}`: {error}"))?;
        }
        Ok(())
    }
}

fn extract_chat(layout: &mut TabLayout, id: &str) {
    layout.extract(id);
    layout.closed.retain(|tab| tab.id != id);
}

fn valid_root(id: &str) -> bool {
    is_chat(id)
        && id
            .split_once(':')
            .is_some_and(|(_, value)| !value.is_empty())
}

#[cfg(test)]
mod tests;
