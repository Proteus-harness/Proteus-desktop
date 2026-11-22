//! Edit provider/model pairs per agent while preserving source references.

use super::Draft;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderSnapshot,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ModelTarget {
    Root,
    Role(String),
}

impl ModelTarget {
    pub fn alias<'a>(&self, draft: &'a Draft) -> Option<&'a str> {
        match self {
            Self::Root => draft.model.as_deref(),
            Self::Role(role) => draft.role_models.get(role)?.as_deref(),
        }
    }

    pub fn configured<'a>(&self, draft: &'a Draft) -> Option<&'a ConfigBuilderModelPreset> {
        draft.models.get(self.alias(draft)?)
    }

    pub fn set_choice(&self, draft: &mut Draft, choice: Option<String>) {
        match self {
            Self::Root => draft.model = choice,
            Self::Role(role) => {
                draft.role_models.insert(role.clone(), choice);
            }
        }
    }

    pub fn choice_writable(&self, snapshot: &ConfigBuilderSnapshot) -> bool {
        snapshot.writable
            && match self {
                Self::Root => snapshot.model_writable,
                Self::Role(name) => snapshot
                    .model_roles
                    .iter()
                    .any(|role| role.name == *name && role.editable),
            }
    }

    fn generated_alias_prefix(&self) -> String {
        match self {
            Self::Root => "model-main".into(),
            Self::Role(role) => format!("model-role-{role}"),
        }
    }

    /// Replace only the selected agent's settings. A shared stored definition
    /// is copied before editing so explicit choices remain independent.
    pub fn set_model(
        &self,
        draft: &mut Draft,
        baseline: &Draft,
        snapshot: &ConfigBuilderSnapshot,
        model: Option<ConfigBuilderModelPreset>,
    ) {
        if !snapshot.models_writable || !self.choice_writable(snapshot) {
            return;
        }
        if model.as_ref() == self.configured(draft) {
            return;
        }
        let previous = self.alias(draft).map(str::to_owned);
        let prefix = self.generated_alias_prefix();
        let choice = model.map(|model| {
            let original = self
                .alias(baseline)
                .filter(|alias| draft.models.get(*alias) == Some(&model));
            let exclusive = previous
                .as_deref()
                .filter(|alias| model_references(draft, snapshot, alias) == 1);
            let alias = original.or(exclusive).map_or_else(
                || {
                    let mut alias = prefix.clone();
                    for suffix in 2.. {
                        if !draft.models.contains_key(&alias) {
                            break;
                        }
                        alias = format!("{prefix}-{suffix}");
                    }
                    alias
                },
                str::to_owned,
            );
            draft.models.insert(alias.clone(), model);
            alias
        });
        self.set_choice(draft, choice);
        if let Some(previous) = previous
            && previous.starts_with(&prefix)
            && !baseline.models.contains_key(&previous)
            && model_references(draft, snapshot, &previous) == 0
        {
            draft.models.remove(&previous);
        }
    }
}

fn model_references(draft: &Draft, snapshot: &ConfigBuilderSnapshot, alias: &str) -> usize {
    draft
        .role_models
        .values()
        .filter(|choice| choice.as_deref() == Some(alias))
        .count()
        + snapshot
            .model_roles
            .iter()
            .filter(|role| !role.editable && role.model.as_deref() == Some(alias))
            .count()
        + usize::from(draft.model.as_deref() == Some(alias))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModelText {
    Model,
    Effort,
    Budget,
    Efforts,
}
impl ModelText {
    pub const ALL: [Self; 4] = [Self::Model, Self::Effort, Self::Budget, Self::Efforts];
    pub fn title(self) -> &'static str {
        match self {
            Self::Model => "ID модели",
            Self::Effort => "Effort",
            Self::Budget => "Бюджет рассуждения",
            Self::Efforts => "Доступные уровни effort",
        }
    }
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Model => "Точный ID модели",
            Self::Effort => "По умолчанию",
            Self::Budget => "Не задано",
            Self::Efforts => "Например: low, medium, high",
        }
    }
}

pub fn model_text(model: &ConfigBuilderModelPreset, field: ModelText) -> String {
    match field {
        ModelText::Model => model.model.clone(),
        ModelText::Effort => model.reasoning.effort.clone().unwrap_or_default(),
        ModelText::Budget => model
            .reasoning
            .budget_tokens
            .map(|value| value.to_string())
            .unwrap_or_default(),
        ModelText::Efforts => model.reasoning_efforts.join(", "),
    }
}

pub fn set_model_text(
    model: &mut ConfigBuilderModelPreset,
    field: ModelText,
    text: &str,
) -> Result<(), String> {
    match field {
        ModelText::Model => model.model = text.to_owned(),
        ModelText::Effort => model.reasoning.effort = (!text.is_empty()).then(|| text.to_owned()),
        ModelText::Budget => {
            model.reasoning.budget_tokens = if text.is_empty() {
                None
            } else {
                Some(
                    text.parse::<u32>()
                        .map_err(|_| "Укажите целое число от 0 до 4294967295".to_owned())?,
                )
            }
        }
        ModelText::Efforts => {
            model.reasoning_efforts = text
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect()
        }
    }
    Ok(())
}

/// Undo restores the exact source representation of only the edited field.
/// Display strings can be lossy (notably comma-separated opaque effort IDs).
pub fn edit_model_text(
    model: &mut ConfigBuilderModelPreset,
    baseline: Option<&ConfigBuilderModelPreset>,
    field: ModelText,
    text: &str,
) -> Result<(), String> {
    if let Some(saved) = baseline.filter(|saved| model_text(saved, field) == text) {
        match field {
            ModelText::Model => model.model = saved.model.clone(),
            ModelText::Effort => model.reasoning.effort = saved.reasoning.effort.clone(),
            ModelText::Budget => model.reasoning.budget_tokens = saved.reasoning.budget_tokens,
            ModelText::Efforts => model.reasoning_efforts = saved.reasoning_efforts.clone(),
        }
        Ok(())
    } else {
        set_model_text(model, field, text)
    }
}

pub fn model_problems(draft: &Draft, snapshot: &ConfigBuilderSnapshot) -> Vec<String> {
    let mut problems = Vec::new();
    if !snapshot.model_writable && draft.model != snapshot.model {
        problems.push(
            "Выбор модели принадлежит другому источнику. Правьте его в исходном конфиге.".into(),
        );
    }
    if !snapshot.models_writable && draft.models != snapshot.models {
        problems.push("Модели принадлежат другому файлу. Правьте их в исходном конфиге.".into());
    }
    for (name, model) in &draft.models {
        let target = if draft.model.as_deref() == Some(name) {
            "Основной агент"
        } else {
            draft
                .role_models
                .iter()
                .find_map(|(role, choice)| {
                    (choice.as_deref() == Some(name)).then_some(role.as_str())
                })
                .unwrap_or(&model.model)
        };
        if name.trim().is_empty() {
            problems.push("У модели должно быть непустое имя".into());
        }
        if model.provider.trim().is_empty() {
            problems.push(format!("{target}: выберите подключение"));
        } else if !snapshot.model_modules.is_empty()
            && !snapshot
                .model_modules
                .iter()
                .any(|module| module.id == model.provider)
        {
            problems.push(format!(
                "{target}: подключение {} недоступно",
                model.provider
            ));
        }
        if model.model.trim().is_empty() {
            problems.push(format!("{target}: укажите ID модели"));
        }
        if model
            .reasoning
            .effort
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            problems.push(format!(
                "{target}: effort должен быть непустым или отсутствовать"
            ));
        }
    }
    if let Some(name) = &draft.model
        && !draft.models.contains_key(name)
    {
        problems.push(format!(
            "Основной агент ссылается на отсутствующую модель {name}"
        ));
    }
    for (role, choice) in &draft.role_models {
        if let Some(name) = choice
            && !draft.models.contains_key(name)
        {
            problems.push(format!("{role}: отсутствующая модель {name}"));
        }
    }
    problems
}
