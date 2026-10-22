use super::control::MenuItem;
use super::widget::Tone;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub label: String,
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
    /// Shows a checkbox; toggling it sends the `check` event.
    #[serde(default)]
    pub checked: Option<bool>,
    /// Draws the label struck through, such as a deleted file.
    #[serde(default)]
    pub strike: bool,
    /// Opened on right click.
    #[serde(default)]
    pub menu: Vec<MenuItem>,
}
