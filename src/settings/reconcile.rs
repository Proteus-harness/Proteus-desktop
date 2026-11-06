//! A catalog write changes saved addons while the settings draft can keep
//! moving. Rebase just that addon delta; unrelated profile fields and editor
//! buffers belong to the newer local draft.

use std::{collections::BTreeSet, path::PathBuf};

use proteus_contracts::app_protocol::addons::AppAddonsUpdate;

#[derive(Clone, Debug)]
pub enum Replacement {
    Plugin { old: PathBuf, new: PathBuf },
    Mcp { old: String, new: String },
}

pub struct CatalogChange {
    pub before: AppAddonsUpdate,
    /// Exact old/new binding supplied by the operation, never guessed from
    /// a manifest name or from similar paths.
    pub replacement: Option<Replacement>,
}

impl CatalogChange {
    /// Keep overlapping local edits dirty and return their names for an
    /// explicit warning. An accepted remote write is still the new baseline.
    pub fn rebase(
        &self,
        current: &mut AppAddonsUpdate,
        accepted: &AppAddonsUpdate,
        edited_mcp: &BTreeSet<String>,
    ) -> Vec<String> {
        let mut conflicts = Vec::new();
        let mut skip_plugins = BTreeSet::new();
        let mut skip_mcp = BTreeSet::new();
        let mut conflicted_mcp = BTreeSet::new();
        let mut preserved_switches = BTreeSet::new();
        let mut protected_mcp = edited_mcp.clone();
        for name in current
            .addons
            .disabled_mcp_servers
            .iter()
            .chain(&self.before.addons.disabled_mcp_servers)
        {
            if current.addons.disabled_mcp_servers.contains(name)
                != self.before.addons.disabled_mcp_servers.contains(name)
            {
                protected_mcp.insert(name.clone());
            }
        }
        match &self.replacement {
            Some(Replacement::Plugin { old, new }) => {
                let before = self
                    .before
                    .addons
                    .plugins
                    .iter()
                    .find(|entry| entry.path == *old);
                let after = accepted
                    .addons
                    .plugins
                    .iter()
                    .find(|entry| entry.path == *new);
                if let (Some(before), Some(after)) = (before, after) {
                    skip_plugins.extend([old.clone(), new.clone()]);
                    let position = current
                        .addons
                        .plugins
                        .iter()
                        .position(|entry| entry.path == *old);
                    let already = current
                        .addons
                        .plugins
                        .iter()
                        .find(|entry| entry.path == *new);
                    match position {
                        Some(ix) if old == new || already.is_none() => {
                            let local = &current.addons.plugins[ix];
                            // The package switch may be changed while the version is
                            // downloading/saving. Carry it to the exact replacement,
                            // together with the accepted stable data identity.
                            if local.data_key == before.data_key {
                                let enabled = if local.enabled != before.enabled {
                                    local.enabled
                                } else {
                                    after.enabled
                                };
                                current.addons.plugins[ix] = after.clone();
                                current.addons.plugins[ix].enabled = enabled;
                            } else {
                                conflicts.push(format!("пакет {}", old.display()));
                            }
                        }
                        None if already == Some(after) => {}
                        _ => conflicts.push(format!("пакет {}", old.display())),
                    }
                }
            }
            Some(Replacement::Mcp { old, new }) if old != new => {
                let before = self
                    .before
                    .mcp_servers
                    .iter()
                    .find(|entry| entry.name == *old);
                let after = accepted.mcp_servers.iter().find(|entry| entry.name == *new);
                if let (Some(before), Some(after)) = (before, after) {
                    skip_mcp.extend([old.clone(), new.clone()]);
                    let position = current
                        .mcp_servers
                        .iter()
                        .position(|entry| entry.name == *old);
                    let already = current.mcp_servers.iter().find(|entry| entry.name == *new);
                    match position {
                        Some(ix)
                            if current.mcp_servers[ix] == *before
                                && !edited_mcp.contains(old)
                                && already.is_none() =>
                        {
                            current.mcp_servers[ix] = after.clone();
                            let disabled = current.addons.disabled_mcp_servers.contains(old);
                            if disabled != self.before.addons.disabled_mcp_servers.contains(old) {
                                current
                                    .addons
                                    .disabled_mcp_servers
                                    .retain(|name| name != old && name != new);
                                if disabled {
                                    current.addons.disabled_mcp_servers.push(new.clone());
                                }
                                preserved_switches.extend([old.clone(), new.clone()]);
                            }
                        }
                        None if already == Some(after) => {}
                        _ => {
                            conflicts.push(format!("MCP {old}"));
                            conflicted_mcp.extend([old.clone(), new.clone()]);
                        }
                    }
                }
            }
            _ => {}
        }
        conflicts.extend(
            merge_entries(
                &mut current.addons.plugins,
                &self.before.addons.plugins,
                &accepted.addons.plugins,
                |entry| entry.path.clone(),
                |key| skip_plugins.contains(key),
                |_| false,
            )
            .into_iter()
            .map(|key| format!("пакет {}", key.display())),
        );
        conflicted_mcp.extend(merge_entries(
            &mut current.mcp_servers,
            &self.before.mcp_servers,
            &accepted.mcp_servers,
            |entry| entry.name.clone(),
            |key| skip_mcp.contains(key),
            |key| protected_mcp.contains(key),
        ));
        conflicts.extend(
            conflicted_mcp
                .iter()
                .filter(|key| !skip_mcp.contains(*key))
                .map(|key| format!("MCP {key}")),
        );
        preserved_switches.extend(conflicted_mcp);
        merge_switches(
            &mut current.addons.disabled_skills,
            &self.before.addons.disabled_skills,
            &accepted.addons.disabled_skills,
            &BTreeSet::new(),
        );
        merge_switches(
            &mut current.addons.disabled_mcp_servers,
            &self.before.addons.disabled_mcp_servers,
            &accepted.addons.disabled_mcp_servers,
            &preserved_switches,
        );
        conflicts
    }
}

/// Lists retain the local order. A saved replacement stays in place, a new
/// entry is appended, and changes to other entries survive a delayed ACK.
fn merge_entries<T: Clone + PartialEq, K: Clone + Ord>(
    current: &mut Vec<T>,
    before: &[T],
    accepted: &[T],
    key: impl Fn(&T) -> K,
    skip: impl Fn(&K) -> bool,
    protected: impl Fn(&K) -> bool,
) -> Vec<K> {
    let mut conflicts = Vec::new();
    let keys: BTreeSet<K> = before.iter().chain(accepted).map(&key).collect();
    for name in keys {
        if skip(&name) {
            continue;
        }
        let old = before.iter().find(|entry| key(entry) == name);
        let saved = accepted.iter().find(|entry| key(entry) == name);
        if old == saved {
            continue;
        }
        let position = current.iter().position(|entry| key(entry) == name);
        let local = position.map(|ix| &current[ix]);
        if local == saved {
            continue;
        }
        if local != old || protected(&name) {
            conflicts.push(name);
            continue;
        }
        match (position, saved) {
            (Some(ix), Some(saved)) => current[ix] = saved.clone(),
            (Some(ix), None) => {
                current.remove(ix);
            }
            (None, Some(saved)) => current.push(saved.clone()),
            (None, None) => {}
        }
    }
    conflicts
}

fn merge_switches(
    current: &mut Vec<String>,
    before: &[String],
    accepted: &[String],
    skip: &BTreeSet<String>,
) {
    let keys: BTreeSet<_> = before.iter().chain(accepted).collect();
    for key in keys {
        if skip.contains(key) {
            continue;
        }
        let (old, saved) = (before.contains(key), accepted.contains(key));
        if old != saved && current.contains(key) == old {
            if saved {
                current.push(key.clone());
            } else {
                current.retain(|entry| entry != key);
            }
        }
    }
    current.sort();
    current.dedup();
}

#[cfg(test)]
mod tests;
