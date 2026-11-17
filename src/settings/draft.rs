use std::collections::{BTreeMap, BTreeSet};

use proteus_contracts::app_protocol::addons::AppAddonsUpdate;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderSnapshot, ConfigBuilderState,
};
use proteus_contracts::app_protocol::http::SetConfigBuilderRequest;
use proteus_contracts::domain::BehaviorModeDefinition;
use serde_json::{Map, Value};

/// Profile area touched by an edit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Area {
    Models,
    Modes,
    Mode,
    Tools,
    Hooks,
    Mcp,
    Skills,
    Packages,
    Slot(String),
}

/// Address of one module config value:
/// `module_config[slot][module][keys[0]][keys[1]]…`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FieldPath {
    pub slot: String,
    pub module: String,
    pub keys: Vec<String>,
}

impl FieldPath {
    pub fn new(slot: &str, module: &str, key: &str) -> Self {
        Self {
            slot: slot.to_string(),
            module: module.to_string(),
            keys: vec![key.to_string()],
        }
    }

    /// Field `key` inside the object at this path.
    pub fn child(&self, key: &str) -> Self {
        let mut path = self.clone();
        path.keys.push(key.to_string());
        path
    }

    /// Whether this path is `prefix` or lies inside it.
    pub fn starts_with(&self, prefix: &FieldPath) -> bool {
        self.slot == prefix.slot
            && self.module == prefix.module
            && self.keys.starts_with(&prefix.keys)
    }

    /// Dotted keys, as written in the profile.
    pub fn dotted(&self) -> String {
        self.keys.join(".")
    }
}

/// Editable copy of the persisted profile settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub modules: BTreeMap<String, String>,
    pub config: BTreeMap<String, BTreeMap<String, Value>>,
    pub hooks: Vec<String>,
    pub tools: BTreeSet<String>,
    pub model: Option<String>,
    pub models: BTreeMap<String, ConfigBuilderModelPreset>,
    pub role_models: BTreeMap<String, Option<String>>,
    pub modes: BTreeMap<String, BehaviorModeDefinition>,
    pub mode: String,
    /// Skills, MCP servers and agent plugin packages.
    pub addons: AppAddonsUpdate,
}

impl Draft {
    pub fn from_snapshot(snapshot: &ConfigBuilderSnapshot) -> Self {
        Self {
            modules: snapshot
                .active_modules
                .iter()
                .map(|selection| (selection.slot.clone(), selection.id.clone()))
                .collect(),
            config: snapshot.module_config.clone(),
            hooks: snapshot.hooks.clone(),
            tools: snapshot.tools_enabled.iter().cloned().collect(),
            model: snapshot.model.clone(),
            models: snapshot.models.clone(),
            modes: snapshot.modes.clone(),
            role_models: snapshot
                .model_roles
                .iter()
                .filter(|role| role.editable)
                .map(|role| (role.name.clone(), role.model.clone()))
                .collect(),
            mode: snapshot.permission_mode.clone(),
            addons: super::addons::normalized(snapshot.addon_settings.clone()),
        }
    }

    pub fn value(&self, path: &FieldPath) -> Option<&Value> {
        let module = self.config.get(&path.slot)?.get(&path.module)?;
        path.keys
            .iter()
            .try_fold(module, |value, key| value.as_object()?.get(key))
    }

    /// Sets a value, creating the objects on its way; `None` removes it and
    /// then the nested objects it leaves empty.
    pub fn set_value(&mut self, path: &FieldPath, value: Option<Value>) {
        let Some((leaf, parents)) = path.keys.split_last() else {
            return;
        };
        match value {
            Some(value) => {
                let mut object = self
                    .config
                    .entry(path.slot.clone())
                    .or_default()
                    .entry(path.module.clone())
                    .or_insert_with(|| Value::Object(Map::new()));
                for key in parents {
                    object = ensure_object(object)
                        .entry(key.clone())
                        .or_insert_with(|| Value::Object(Map::new()));
                }
                ensure_object(object).insert(leaf.clone(), value);
            }
            None => {
                if let Some(module) = self
                    .config
                    .get_mut(&path.slot)
                    .and_then(|modules| modules.get_mut(&path.module))
                {
                    remove_nested(module, &path.keys, None);
                }
            }
        }
    }

    /// Schema controls compare effective values, but returning to the saved
    /// value must also restore its original explicit/omitted representation.
    /// Only the edited leaf and empty ancestors on its path are restored.
    pub fn edit_value(
        &mut self,
        path: &FieldPath,
        value: Option<Value>,
        default: Option<&Value>,
        baseline: &Draft,
    ) {
        let saved = baseline.value(path);
        let value = if value.as_ref().or(default) == saved.or(default) {
            saved.cloned()
        } else {
            value.filter(|value| Some(value) != default)
        };
        if value.is_some() {
            self.set_value(path, value);
        } else if let Some(module) = self
            .config
            .get_mut(&path.slot)
            .and_then(|modules| modules.get_mut(&path.module))
        {
            remove_nested(
                module,
                &path.keys,
                baseline.module_config(&path.slot, &path.module),
            );
        }
    }

    pub fn set_module(&mut self, slot: &str, module: &str) {
        self.modules.insert(slot.to_string(), module.to_string());
    }

    pub fn set_hook(&mut self, id: &str, enabled: bool) {
        if enabled {
            if !self.hooks.iter().any(|hook| hook == id) {
                self.hooks.push(id.to_string());
            }
        } else {
            self.hooks.retain(|hook| hook != id);
        }
    }

    pub fn set_tool(&mut self, name: &str, enabled: bool) {
        if enabled {
            self.tools.insert(name.to_string());
        } else {
            self.tools.remove(name);
        }
    }

    /// Whole parameter object of one module (`module_config[slot][module]`).
    pub fn module_config(&self, slot: &str, module: &str) -> Option<&Value> {
        self.config.get(slot)?.get(module)
    }

    pub fn set_module_config(&mut self, slot: &str, module: &str, value: Map<String, Value>) {
        self.config
            .entry(slot.to_string())
            .or_default()
            .insert(module.to_string(), Value::Object(value));
    }

    /// Draft that would bring the profile back to a replaced history state.
    /// The builder cannot clear a slot selection, so slots missing from the
    /// old state keep their current module; modules configured now but absent
    /// from the old state get an explicit empty object.
    pub fn from_revision(snapshot: &ConfigBuilderSnapshot, state: &ConfigBuilderState) -> Self {
        let mut draft = Self::from_snapshot(snapshot);
        for selection in &state.active_modules {
            draft
                .modules
                .insert(selection.slot.clone(), selection.id.clone());
        }
        let mut config = state.module_config.clone();
        for (slot, modules) in &snapshot.module_config {
            let target = config.entry(slot.clone()).or_default();
            for (module, value) in modules {
                target.entry(module.clone()).or_insert_with(|| {
                    if slot == "model" && !state.model_connections.contains(module) {
                        // Installing a component is not undone by settings
                        // history. Its required seed must stay usable.
                        value.clone()
                    } else {
                        Value::Object(Map::new())
                    }
                });
            }
        }
        draft.config = config;
        draft.hooks = state.hooks.clone();
        draft.tools = state.tools_enabled.iter().cloned().collect();
        if snapshot.model_writable {
            draft.model = state.model.clone();
        }
        if snapshot.models_writable {
            draft.models = state.models.clone();
        }
        if snapshot.modes_writable {
            draft.modes = state.modes.clone();
        }
        for (name, choice) in &mut draft.role_models {
            if let Some(saved) = state.role_models.get(name) {
                *choice = saved.clone();
            }
        }
        draft.mode = state.permission_mode.clone();
        draft.addons = super::addons::normalized(state.addon_settings.clone());
        draft
    }

    /// Profile areas that differ from `baseline`, in a stable order.
    pub fn areas(&self, baseline: &Draft) -> Vec<Area> {
        let request = self.changes(baseline);
        let mut areas = BTreeSet::new();
        if request.model.is_some() || request.models.is_some() || request.role_models.is_some() {
            areas.insert(Area::Models);
        }
        if request.permission_mode.is_some() {
            areas.insert(Area::Mode);
        }
        if request.modes.is_some() {
            areas.insert(Area::Modes);
        }
        if request.tools_enabled.is_some() {
            areas.insert(Area::Tools);
        }
        if request.hooks.is_some() {
            areas.insert(Area::Hooks);
        }
        let (before, after) = (&baseline.addons, &self.addons);
        if before.mcp_servers != after.mcp_servers
            || before.addons.disabled_mcp_servers != after.addons.disabled_mcp_servers
        {
            areas.insert(Area::Mcp);
        }
        if before.addons.disabled_skills != after.addons.disabled_skills {
            areas.insert(Area::Skills);
        }
        if before.addons.plugins != after.addons.plugins {
            areas.insert(Area::Packages);
        }
        for slot in request.modules.keys().chain(request.module_config.keys()) {
            areas.insert(Area::Slot(slot.clone()));
        }
        areas.into_iter().collect()
    }

    /// Minimal save request: only sections that differ from `baseline`.
    /// The server replaces a whole slot map in `module_config`, so a changed
    /// slot is sent in full.
    pub fn changes(&self, baseline: &Draft) -> SetConfigBuilderRequest {
        let mut request = SetConfigBuilderRequest::default();
        for (slot, module) in &self.modules {
            if baseline.modules.get(slot) != Some(module) {
                request.modules.insert(slot.clone(), module.clone());
            }
        }
        let empty = BTreeMap::new();
        for (slot, modules) in &self.config {
            let before = baseline.config.get(slot).unwrap_or(&empty);
            let after: BTreeMap<String, Value> = modules
                .iter()
                .filter(|(module, value)| before.contains_key(*module) || !is_empty_object(value))
                .map(|(module, value)| (module.clone(), value.clone()))
                .collect();
            if &after != before {
                request.module_config.insert(slot.clone(), after);
            }
        }
        if self.hooks != baseline.hooks {
            request.hooks = Some(self.hooks.clone());
        }
        if self.tools != baseline.tools {
            request.tools_enabled = Some(self.tools.iter().cloned().collect());
        }
        if self.model != baseline.model {
            request.model = Some(self.model.clone());
        }
        if self.models != baseline.models {
            request.models = Some(self.models.clone());
            request.expected_models = Some(baseline.models.clone());
        }
        if self.modes != baseline.modes {
            request.modes = Some(self.modes.clone());
            request.expected_modes = Some(baseline.modes.clone());
        }
        let roles = self
            .role_models
            .iter()
            .filter(|(role, model)| baseline.role_models.get(*role) != Some(*model))
            .map(|(role, model)| (role.clone(), model.clone()))
            .collect::<BTreeMap<_, _>>();
        if !roles.is_empty() {
            request.role_models = Some(roles);
        }
        if self.mode != baseline.mode {
            request.permission_mode = serde_json::from_value(Value::String(self.mode.clone())).ok();
        }
        if self.addons != baseline.addons {
            request.addon_settings = Some(self.addons.clone());
        }
        request
    }

    pub fn is_dirty(&self, baseline: &Draft) -> bool {
        let request = self.changes(baseline);
        !request.modules.is_empty()
            || !request.module_config.is_empty()
            || request.hooks.is_some()
            || request.tools_enabled.is_some()
            || request.model.is_some()
            || request.models.is_some()
            || request.modes.is_some()
            || request.role_models.is_some()
            || request.permission_mode.is_some()
            || request.addon_settings.is_some()
    }
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_some_and(Map::is_empty)
}

/// Object at `value`, replacing a scalar that stands in its way.
fn ensure_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value
        .as_object_mut()
        .expect("value was just made an object")
}

/// Removes `keys` under `value`; returns whether the caller should prune the
/// empty object. Schema edits and their undo retain empty saved ancestors.
fn remove_nested(value: &mut Value, keys: &[String], baseline: Option<&Value>) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    match keys {
        [] => false,
        [leaf] => {
            object.remove(leaf);
            object.is_empty() && !baseline.is_some_and(is_empty_object)
        }
        [key, rest @ ..] => {
            if object.get_mut(key).is_some_and(|child| {
                remove_nested(child, rest, baseline.and_then(|value| value.get(key)))
            }) {
                object.remove(key);
            }
            object.is_empty() && !baseline.is_some_and(is_empty_object)
        }
    }
}
