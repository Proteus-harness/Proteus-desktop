//! Pieces shared by interactive nodes: menus and confirmations.

use serde::Deserialize;
use serde_json::Value;

/// One entry of a dropdown or context menu; picking it sends `action`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    pub label: String,
    pub action: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub danger: bool,
    #[serde(default)]
    pub confirm: Option<Confirm>,
    /// Draws a separator above the entry.
    #[serde(default)]
    pub separated: bool,
}

/// Asks the user before the action is sent.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Confirm {
    pub title: String,
    #[serde(default)]
    pub message: Option<String>,
    /// Label of the confirming button.
    #[serde(default)]
    pub ok: Option<String>,
    /// Paints the confirming button as destructive.
    #[serde(default)]
    pub danger: bool,
}
