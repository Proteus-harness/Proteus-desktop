use super::control::MenuItem;
use super::widget::Tone;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub label: String,
    /// Muted text after the label, cut from its start when short of room,
    /// such as the folder of a file.
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub depth: usize,
    #[serde(default)]
    pub expanded: Option<bool>,
    #[serde(default)]
    pub tone: Tone,
    #[serde(default)]
    pub selected: bool,
    /// Lucide icon name; folders and files get a default one.
    #[serde(default)]
    pub icon: Option<String>,
    /// Tone of the icon alone; the row tone otherwise.
    #[serde(default)]
    pub icon_tone: Option<Tone>,
    /// Shows a checkbox at the end of the row; toggling it sends the
    /// `check` event.
    #[serde(default)]
    pub checked: Option<Check>,
    /// Draws the label struck through, such as a deleted file.
    #[serde(default)]
    pub strike: bool,
    /// Opened on right click.
    #[serde(default)]
    pub menu: Vec<MenuItem>,
    #[serde(default)]
    pub kind: Kind,
    /// A small counter chip at the end of the row.
    #[serde(default)]
    pub count: Option<usize>,
    /// Added and deleted lines, at the end of the row.
    #[serde(default)]
    pub stats: Option<Stats>,
    /// Hover hint, such as the full path.
    #[serde(default)]
    pub hint: Option<String>,
}

impl Row {
    pub fn header(&self) -> bool {
        self.kind == Kind::Header
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Item,
    /// A section title: a click folds it, the keyboard passes it by.
    Header,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stats {
    pub added: u64,
    pub deleted: u64,
}

/// State of a row checkbox: JSON `true`, `false` or `"mixed"`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Off,
    On,
    /// Part of what the row stands for is checked.
    Mixed,
}

impl<'de> Deserialize<'de> for Check {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Word(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Flag(true) => Ok(Check::On),
            Raw::Flag(false) => Ok(Check::Off),
            Raw::Word(word) if word == "mixed" => Ok(Check::Mixed),
            Raw::Word(word) => Err(serde::de::Error::custom(format!(
                "checked {word:?} is neither a boolean nor \"mixed\""
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_tri_state_checks_and_headers() {
        let row: Row = serde_json::from_value(json!({
            "id": "s", "label": "Changes", "kind": "header", "expanded": true,
            "checked": "mixed", "count": 3
        }))
        .unwrap();
        assert!(row.header());
        assert_eq!((row.checked, row.count), (Some(Check::Mixed), Some(3)));
        let row: Row = serde_json::from_value(json!({
            "id": "a", "label": "a.rs", "checked": true,
            "stats": {"added": 4, "deleted": 1}, "hint": "/work/a.rs"
        }))
        .unwrap();
        assert_eq!(row.checked, Some(Check::On));
        assert_eq!(
            row.stats,
            Some(Stats {
                added: 4,
                deleted: 1
            })
        );
        for bad in [
            json!({"id": "a", "label": "a", "checked": "half"}),
            json!({"id": "a", "label": "a", "kind": "footer"}),
            json!({"id": "a", "label": "a", "stats": {"added": 1}}),
            json!({"id": "a", "label": "a", "stats": {"added": -1, "deleted": 0}}),
        ] {
            assert!(serde_json::from_value::<Row>(bad).is_err());
        }
    }
}
