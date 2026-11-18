use proteus_contracts::domain::{ConfigChoice, ConfigField, ConfigValueSchema};
use serde_json::Value;

/// Presentation sections preserve both the first occurrence order and real
/// configuration paths. They do not introduce JSON objects or copy values.
pub struct FieldSection<'a> {
    pub metadata: Option<&'a proteus_contracts::domain::ConfigSection>,
    pub fields: Vec<&'a ConfigField>,
}

pub fn field_sections(fields: &[ConfigField]) -> Vec<FieldSection<'_>> {
    let mut sections: Vec<FieldSection<'_>> = Vec::new();
    for field in fields
        .iter()
        .filter(|field| super::catalog::editable_field(field))
    {
        let id = field.section.as_ref().map(|section| section.id.as_str());
        if let Some(section) = sections
            .iter_mut()
            .find(|section| section.metadata.map(|metadata| metadata.id.as_str()) == id)
        {
            section.fields.push(field);
        } else {
            sections.push(FieldSection {
                metadata: field.section.as_ref(),
                fields: vec![field],
            });
        }
    }
    sections
}

fn push_scope_part(out: &mut String, part: &str) {
    for ch in part.chars() {
        match ch {
            '%' => out.push_str("%25"),
            ':' => out.push_str("%3A"),
            '.' => out.push_str("%2E"),
            _ => out.push(ch),
        }
    }
}

/// Display paths can contain separators; encode each component before joining.
pub fn form_scope(slot: &str, module: &str, keys: &[String]) -> String {
    let mut out = String::new();
    push_scope_part(&mut out, slot);
    out.push(':');
    push_scope_part(&mut out, module);
    for (index, key) in keys.iter().enumerate() {
        out.push(if index == 0 { ':' } else { '.' });
        push_scope_part(&mut out, key);
    }
    out
}

pub fn section_scope(scope: &str, field: &ConfigField) -> String {
    field.section.as_ref().map_or_else(
        || scope.to_owned(),
        |section| {
            let mut out = format!("{scope}:section:");
            push_scope_part(&mut out, &section.id);
            out
        },
    )
}

/// How a text control reads and writes one module config value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextFormat {
    Integer {
        minimum: Option<i64>,
        maximum: Option<i64>,
    },
    Number,
    Line,
    Secret,
    Multiline,
    /// Array of single-line strings, one per line.
    Lines,
    /// Exact list of multiline strings, entered as a JSON array.
    StringArrayJson,
    Json,
    /// JSON on one line for short values like `100000` or `80%`; text that
    /// is not JSON is taken as a string.
    Value,
}

impl TextFormat {
    pub fn is_multiline(self) -> bool {
        matches!(
            self,
            Self::Multiline | Self::Lines | Self::StringArrayJson | Self::Json
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub enum FieldKind<'a> {
    Switch,
    Choice(&'a [ConfigChoice]),
    MultiChoice(&'a [ConfigChoice]),
    Text(TextFormat),
    /// Object with known fields, edited as nested fields.
    Group(&'a [ConfigField]),
}

/// JSON fields with a number or text default take the one-line `Value`
/// format: such values are short.
pub fn field_kind(field: &ConfigField) -> FieldKind<'_> {
    match schema_kind(&field.value) {
        FieldKind::Text(TextFormat::Json)
            if field
                .default
                .as_ref()
                .is_some_and(|default| default.is_number() || default.is_string()) =>
        {
            FieldKind::Text(TextFormat::Value)
        }
        kind => kind,
    }
}

fn schema_kind(schema: &ConfigValueSchema) -> FieldKind<'_> {
    match schema {
        ConfigValueSchema::Boolean {} => FieldKind::Switch,
        ConfigValueSchema::Enum { options } => FieldKind::Choice(options),
        ConfigValueSchema::Integer { minimum, maximum } => FieldKind::Text(TextFormat::Integer {
            minimum: *minimum,
            maximum: *maximum,
        }),
        ConfigValueSchema::Number {} => FieldKind::Text(TextFormat::Number),
        ConfigValueSchema::String { secret: true, .. } => FieldKind::Text(TextFormat::Secret),
        ConfigValueSchema::String {
            multiline: true, ..
        } => FieldKind::Text(TextFormat::Multiline),
        ConfigValueSchema::String { .. } => FieldKind::Text(TextFormat::Line),
        ConfigValueSchema::Array { items } => match items.as_ref() {
            ConfigValueSchema::Enum { options } => FieldKind::MultiChoice(options),
            ConfigValueSchema::String {
                multiline: false,
                secret: false,
            } => FieldKind::Text(TextFormat::Lines),
            ConfigValueSchema::String {
                multiline: true,
                secret: false,
            } => FieldKind::Text(TextFormat::StringArrayJson),
            _ => FieldKind::Text(TextFormat::Json),
        },
        ConfigValueSchema::Object { fields } if !fields.is_empty() => FieldKind::Group(fields),
        ConfigValueSchema::Object { .. } | ConfigValueSchema::Json {} => {
            FieldKind::Text(TextFormat::Json)
        }
    }
}

pub fn choice_title<'a>(options: &'a [ConfigChoice], value: &Value) -> Option<&'a str> {
    options
        .iter()
        .find(|choice| &choice.value == value)
        .map(|choice| choice.title.as_str())
}

pub fn format_value(format: TextFormat, value: &Value) -> String {
    match (format, value) {
        // Leave strings bare only when re-reading them preserves their value.
        (TextFormat::Value, Value::String(text))
            if !text.is_empty()
                && text.trim() == text
                && serde_json::from_str::<Value>(text).is_err() =>
        {
            text.clone()
        }
        (_, Value::String(text))
            if !matches!(
                format,
                TextFormat::Json | TextFormat::StringArrayJson | TextFormat::Value
            ) =>
        {
            text.clone()
        }
        (TextFormat::Lines, Value::Array(items)) if items.iter().all(Value::is_string) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        (TextFormat::Integer { .. } | TextFormat::Number, Value::Number(number)) => {
            number.to_string()
        }
        (
            TextFormat::Json
            | TextFormat::StringArrayJson
            | TextFormat::Lines
            | TextFormat::Multiline,
            _,
        ) => serde_json::to_string_pretty(value).unwrap_or_default(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// Parses control text into a config value. Blank text means "not set", so
/// the module falls back to its own default.
pub fn parse_text(format: TextFormat, text: &str) -> Result<Option<Value>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match format {
        TextFormat::Integer { minimum, maximum } => {
            let number = trimmed
                .parse::<i64>()
                .map_err(|_| "Нужно целое число".to_string())?;
            if let Some(minimum) = minimum.filter(|minimum| number < *minimum) {
                return Err(format!("Не меньше {minimum}"));
            }
            if let Some(maximum) = maximum.filter(|maximum| number > *maximum) {
                return Err(format!("Не больше {maximum}"));
            }
            Ok(Some(Value::from(number)))
        }
        TextFormat::Number => trimmed
            .parse::<f64>()
            .ok()
            .and_then(|number| {
                // Preserve the existing representation unless binary64 would round
                // an integer spelling that JSON can store exactly.
                if let Ok(whole) = trimmed.parse::<i128>()
                    && whole != number as i128
                    && let Some(exact) = serde_json::Number::from_i128(whole)
                {
                    return Some(exact);
                }
                serde_json::Number::from_f64(number)
            })
            .map(|number| Some(Value::Number(number)))
            .ok_or_else(|| "Нужно число".to_string()),
        TextFormat::Line | TextFormat::Secret | TextFormat::Multiline => {
            Ok(Some(Value::String(text.to_string())))
        }
        TextFormat::Lines => Ok(Some(Value::Array(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(|line| Value::String(line.to_string()))
                .collect(),
        ))),
        TextFormat::Json | TextFormat::StringArrayJson => {
            match serde_json::from_str::<Value>(trimmed) {
                Ok(Value::Null) => Ok(None),
                Ok(value)
                    if format == TextFormat::StringArrayJson
                        && !value
                            .as_array()
                            .is_some_and(|items| items.iter().all(Value::is_string)) =>
                {
                    Err("Нужен JSON-массив строк".to_string())
                }
                Ok(value) => Ok(Some(value)),
                Err(error) => Err(format!("Неверный JSON: {error}")),
            }
        }
        TextFormat::Value => match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Null) => Ok(None),
            Ok(value) => Ok(Some(value)),
            Err(_) => Ok(Some(Value::String(trimmed.to_string()))),
        },
    }
}

#[cfg(test)]
mod value_roundtrip_tests {
    use super::*;

    #[test]
    fn compact_json_text_preserves_strings_on_the_next_edit() {
        let format = TextFormat::Value;
        for text in [
            "",
            " ",
            "\u{202f}°C",
            "Итого: ",
            "\u{202f}42\u{202f}",
            "\u{202f}null\u{202f}",
        ] {
            let original = Value::String(text.into());
            let shown = format_value(format, &original);
            assert_eq!(parse_text(format, &shown), Ok(Some(original)), "{text:?}");
        }
    }
}
