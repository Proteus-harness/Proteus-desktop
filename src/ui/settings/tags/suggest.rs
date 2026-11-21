//! First visible tag suggestions, in schema/registry order, without building
//! owned rows for the hidden part of the catalog.

use std::collections::HashSet;

use proteus_contracts::app_protocol::config_builder::ConfigBuilderSnapshot;
use proteus_contracts::domain::{ConfigField, ConfigSuggestions, ConfigValueSchema};
use serde_json::Value;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Suggestion {
    pub value: String,
    pub title: String,
}

pub(super) fn visible(
    field: &ConfigField,
    snapshot: Option<&ConfigBuilderSnapshot>,
    selected: &[Value],
    query: &str,
) -> Vec<Suggestion> {
    let query = query.to_lowercase();
    let mut seen = HashSet::new();
    candidates(field, snapshot)
        // Keep the first title for a value even if its title does not match.
        .filter(|&(value, _)| seen.insert(value))
        .filter(|&(value, title)| {
            !selected.iter().any(|item| item.as_str() == Some(value))
                && (query.is_empty()
                    || value.to_lowercase().contains(&query)
                    || (title != value && title.to_lowercase().contains(&query)))
        })
        .take(8)
        .map(|(value, title)| Suggestion {
            value: value.into(),
            title: title.into(),
        })
        .collect()
}

fn candidates<'a>(
    field: &'a ConfigField,
    snapshot: Option<&'a ConfigBuilderSnapshot>,
) -> impl Iterator<Item = (&'a str, &'a str)> {
    let options = match &field.value {
        ConfigValueSchema::Array { items } => match items.as_ref() {
            ConfigValueSchema::Enum { options } => options.as_slice(),
            _ => &[],
        },
        _ => &[],
    };
    let options = options
        .iter()
        .filter_map(|option| Some((option.value.as_str()?, option.title.as_str())));
    // Defaults include module-owned values not exposed as exports.
    let defaults = field
        .default
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|value| (value, value));
    let tools = snapshot
        .filter(|_| matches!(field.suggestions, Some(ConfigSuggestions::Tools {})))
        .into_iter()
        .flat_map(|snapshot| &snapshot.tools)
        .map(|tool| (tool.name.as_str(), tool.name.as_str()));
    let exports = match (&field.suggestions, snapshot) {
        (Some(ConfigSuggestions::Exports { slot }), Some(snapshot)) => Some((slot, snapshot)),
        _ => None,
    };
    let exports = exports.into_iter().flat_map(|(slot, snapshot)| {
        let modules = snapshot
            .slots
            .iter()
            .flat_map(|slot| &slot.modules)
            .chain(snapshot.model_modules.iter())
            .chain(snapshot.hook_modules.iter())
            .filter(move |module| &module.slot == slot)
            .map(|module| (module.id.as_str(), module.id.as_str()));
        let plugins = snapshot
            .plugins
            .iter()
            .flat_map(|plugin| &plugin.exports)
            .filter(move |export| &export.slot == slot)
            .map(|export| (export.id.as_str(), export.id.as_str()));
        modules.chain(plugins)
    });
    options.chain(defaults).chain(tools).chain(exports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_title_wins_before_filtering_and_only_eight_rows_are_returned() {
        let mut field = ConfigField::new(
            "tags",
            "Tags",
            "",
            ConfigValueSchema::Array {
                items: Box::new(ConfigValueSchema::choices(&[
                    ("duplicate", "Initial"),
                    ("duplicate", "matches only the later title"),
                ])),
            },
        );
        field.default = Some(serde_json::json!([
            "duplicate",
            "one",
            "two",
            "three",
            "four",
            "five",
            "six",
            "seven",
            "eight",
            "nine"
        ]));
        let selected = [Value::String("one".into()), Value::from(42)];
        assert!(visible(&field, None, &selected, "later").is_empty());
        let rows = visible(&field, None, &selected, "");
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[0].title, "Initial");
        assert_eq!(
            rows.iter()
                .map(|row| row.value.as_str())
                .collect::<Vec<_>>(),
            [
                "duplicate",
                "two",
                "three",
                "four",
                "five",
                "six",
                "seven",
                "eight"
            ]
        );
    }
}
