//! Behavior-mode catalogue edits. IDs and inherited lists retain their exact
//! source representation; only the active chat chooses a named mode.

use std::collections::BTreeSet;

use proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot;
use proteus_contracts::domain::{BehaviorModeDefinition, PermissionMode};

use super::Draft;

/// Choices belong to the connected server. In particular, older servers
/// advertise legacy `auto` and must never receive a new `full_access` choice.
pub fn permission_choices(snapshot: &ConfigBuilderSnapshot) -> Vec<PermissionMode> {
    snapshot
        .permission_modes
        .iter()
        .filter_map(|value| serde_json::from_value(serde_json::Value::String(value.clone())).ok())
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeText {
    Title,
    Instructions,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeList {
    Tools,
    Hooks,
}

impl ModeList {
    pub fn key(self) -> &'static str {
        match self {
            Self::Tools => "tools",
            Self::Hooks => "hooks",
        }
    }
    pub fn values(self, mode: &BehaviorModeDefinition) -> Option<&Vec<String>> {
        match self {
            Self::Tools => mode.tools.as_ref(),
            Self::Hooks => mode.hooks.as_ref(),
        }
    }
    pub fn set(self, mode: &mut BehaviorModeDefinition, value: Option<Vec<String>>) {
        match self {
            Self::Tools => mode.tools = value,
            Self::Hooks => mode.hooks = value,
        }
    }
    pub fn baseline(self, draft: &Draft) -> Vec<String> {
        match self {
            Self::Tools => draft.tools.iter().cloned().collect(),
            Self::Hooks => draft.hooks.clone(),
        }
    }
}

pub fn id_problem(draft: &Draft, id: &str, except: Option<&str>) -> Option<String> {
    if id.trim().is_empty() {
        Some("Укажите имя режима".into())
    } else if id == "_default" {
        Some("Имя _default зарезервировано".into())
    } else if Some(id) != except && draft.modes.contains_key(id) {
        Some("Это имя уже занято".into())
    } else {
        None
    }
}

pub fn text(mode: &BehaviorModeDefinition, field: ModeText) -> &str {
    match field {
        ModeText::Title => mode.title.as_deref(),
        ModeText::Instructions => mode.instructions.as_deref(),
    }
    .unwrap_or("")
}

pub fn set_text(
    mode: &mut BehaviorModeDefinition,
    baseline: Option<&BehaviorModeDefinition>,
    field: ModeText,
    value: &str,
) {
    let saved = baseline.and_then(|mode| match field {
        ModeText::Title => mode.title.as_ref(),
        ModeText::Instructions => mode.instructions.as_ref(),
    });
    let value = if saved.map_or("", String::as_str) == value {
        saved.cloned()
    } else if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    };
    match field {
        ModeText::Title => mode.title = value,
        ModeText::Instructions => mode.instructions = value,
    }
}

impl Draft {
    pub fn add_mode(&mut self, id: &str) -> Result<(), String> {
        if let Some(problem) = id_problem(self, id, None) {
            return Err(problem);
        }
        self.modes
            .insert(id.into(), BehaviorModeDefinition::default());
        Ok(())
    }
    pub fn rename_mode(&mut self, from: &str, to: &str) -> Result<(), String> {
        if let Some(problem) = id_problem(self, to, Some(from)) {
            return Err(problem);
        }
        if !self.modes.contains_key(from) {
            return Err("Режим уже удалён".into());
        }
        if from == to {
            return Ok(());
        }
        let mode = self
            .modes
            .remove(from)
            .ok_or_else(|| "Режим уже удалён".to_string())?;
        self.modes.insert(to.into(), mode);
        Ok(())
    }
}

pub fn problems(draft: &Draft) -> Vec<String> {
    let mut problems = Vec::new();
    for (id, mode) in &draft.modes {
        if let Some(problem) = id_problem(draft, id, Some(id)) {
            problems.push(format!("{id}: {problem}"));
        }
        if mode
            .title
            .as_ref()
            .is_some_and(|title| title.trim().is_empty())
        {
            problems.push(format!("{id}: название не должно быть пустым"));
        }
        for kind in [ModeList::Tools, ModeList::Hooks] {
            if let Some(values) = kind.values(mode) {
                let mut seen = BTreeSet::new();
                for value in values {
                    if value.trim().is_empty() {
                        problems.push(format!("{id} / {}: пустое значение", kind.key()));
                    } else if !seen.insert(value) {
                        problems.push(format!("{id} / {}: повтор {value}", kind.key()));
                    }
                }
            }
        }
    }
    problems
}
