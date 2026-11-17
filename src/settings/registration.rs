//! A connection registration starts from a clean draft. Its authoritative
//! snapshot may also contain other windows' writes: replay only newer local
//! edits over it before accepting the new baseline. Ordinary Save ACKs have
//! different undo semantics and deliberately do not use this merge.

use std::collections::{BTreeMap, BTreeSet};

use proteus_contracts::{
    app_protocol::config_builder::ConfigBuilderModelPreset, domain::BehaviorModeDefinition,
};
use serde_json::Value;

use super::Draft;

impl Draft {
    /// Addons are rebased separately by `CatalogChange`, which also accounts
    /// for edited raw MCP forms and exact package replacements.
    pub fn rebase_registration(&mut self, previous: &Draft, accepted: &Draft) {
        scalar(&mut self.model, &previous.model, &accepted.model);
        scalar(&mut self.mode, &previous.mode, &accepted.mode);
        scalar(&mut self.hooks, &previous.hooks, &accepted.hooks);
        map(
            &mut self.modules,
            &previous.modules,
            &accepted.modules,
            |_, _, _| {},
        );
        map(
            &mut self.role_models,
            &previous.role_models,
            &accepted.role_models,
            |_, _, _| {},
        );
        map(
            &mut self.models,
            &previous.models,
            &accepted.models,
            |local, old, saved| {
                if let Some(old) = old {
                    model(local, old, saved);
                }
            },
        );
        map(
            &mut self.modes,
            &previous.modes,
            &accepted.modes,
            |local, old, saved| {
                if let Some(old) = old {
                    mode(local, old, saved);
                }
            },
        );
        map(
            &mut self.config,
            &previous.config,
            &accepted.config,
            |local, old, saved| {
                map(local, old.unwrap_or(&BTreeMap::new()), saved, json);
            },
        );
        for key in previous.tools.union(&accepted.tools) {
            if self.tools.contains(key) == previous.tools.contains(key) {
                if accepted.tools.contains(key) {
                    self.tools.insert(key.clone());
                } else {
                    self.tools.remove(key);
                }
            }
        }
    }
}

fn scalar<T: Clone + PartialEq>(local: &mut T, old: &T, saved: &T) {
    if local == old {
        *local = saved.clone();
    }
}

/// Only retained entries need field-level merging. A local deletion or a
/// conflicting local creation stays local; accepted additions are preserved.
fn map<T: Clone + PartialEq>(
    local: &mut BTreeMap<String, T>,
    old: &BTreeMap<String, T>,
    saved: &BTreeMap<String, T>,
    merge: impl Fn(&mut T, Option<&T>, &T),
) {
    let keys: BTreeSet<_> = old.keys().chain(saved.keys()).collect();
    for key in keys {
        if local.get(key) == old.get(key) {
            match saved.get(key) {
                Some(value) => {
                    local.insert(key.clone(), value.clone());
                }
                None => {
                    local.remove(key);
                }
            }
        } else if let (Some(local), Some(saved)) = (local.get_mut(key), saved.get(key)) {
            merge(local, old.get(key), saved);
        }
    }
}

fn model(
    local: &mut ConfigBuilderModelPreset,
    old: &ConfigBuilderModelPreset,
    saved: &ConfigBuilderModelPreset,
) {
    scalar(&mut local.provider, &old.provider, &saved.provider);
    scalar(&mut local.model, &old.model, &saved.model);
    scalar(&mut local.stream, &old.stream, &saved.stream);
    scalar(
        &mut local.reasoning.effort,
        &old.reasoning.effort,
        &saved.reasoning.effort,
    );
    scalar(
        &mut local.reasoning.summary,
        &old.reasoning.summary,
        &saved.reasoning.summary,
    );
    scalar(
        &mut local.reasoning.budget_tokens,
        &old.reasoning.budget_tokens,
        &saved.reasoning.budget_tokens,
    );
    scalar(
        &mut local.reasoning_efforts,
        &old.reasoning_efforts,
        &saved.reasoning_efforts,
    );
}

fn mode(
    local: &mut BehaviorModeDefinition,
    old: &BehaviorModeDefinition,
    saved: &BehaviorModeDefinition,
) {
    scalar(&mut local.title, &old.title, &saved.title);
    scalar(
        &mut local.instructions,
        &old.instructions,
        &saved.instructions,
    );
    scalar(&mut local.tools, &old.tools, &saved.tools);
    scalar(&mut local.hooks, &old.hooks, &saved.hooks);
    scalar(
        &mut local.permission_mode,
        &old.permission_mode,
        &saved.permission_mode,
    );
}

fn json(local: &mut Value, old: Option<&Value>, saved: &Value) {
    if Some(&*local) == old {
        *local = saved.clone();
        return;
    }
    let empty = serde_json::Map::new();
    let old = match old {
        Some(Value::Object(old)) => old,
        None => &empty,
        _ => return,
    };
    if let (Value::Object(local), Value::Object(saved)) = (local, saved) {
        let keys: BTreeSet<_> = old.keys().chain(saved.keys()).collect();
        for key in keys {
            if local.get(key) == old.get(key) {
                match saved.get(key) {
                    Some(value) => {
                        local.insert(key.clone(), value.clone());
                    }
                    None => {
                        local.remove(key);
                    }
                }
            } else if let (Some(local), Some(saved)) = (local.get_mut(key), saved.get(key)) {
                json(local, old.get(key), saved);
            }
        }
    }
}

#[cfg(test)]
mod tests;
