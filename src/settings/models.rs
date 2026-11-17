//! Named model presets are reusable values; connection modules and source-role
//! references remain separate identities.

use super::Draft;
use proteus_contracts::app_protocol::config_builder::{
    ConfigBuilderModelPreset, ConfigBuilderSnapshot,
};

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

pub fn new_model_name_problem(draft: &Draft, name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("Укажите имя набора")
    } else if name.trim() != name || name.chars().any(char::is_control) {
        Some("Уберите пробелы по краям и управляющие символы")
    } else if draft.models.contains_key(name) {
        Some("Это имя уже занято")
    } else {
        None
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
        problems.push(
            "Именованные модели принадлежат другому файлу. Правьте их в исходном конфиге.".into(),
        );
    }
    for (name, model) in &draft.models {
        if name.trim().is_empty() {
            problems.push("У модели должно быть непустое имя".into());
        }
        if model.provider.trim().is_empty() {
            problems.push(format!("{name}: выберите подключение"));
        } else if !snapshot.model_modules.is_empty()
            && !snapshot
                .model_modules
                .iter()
                .any(|module| module.id == model.provider)
        {
            problems.push(format!("{name}: подключение {} недоступно", model.provider));
        }
        if model.model.trim().is_empty() {
            problems.push(format!("{name}: укажите ID модели"));
        }
        if model
            .reasoning
            .effort
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
        {
            problems.push(format!(
                "{name}: effort должен быть непустым или отсутствовать"
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
