//! Pieces shared by interactive nodes: menus, confirmations and prompts.

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

/// One entry of a dropdown or context menu; picking it sends `action`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MenuItem {
    pub label: String,
    /// Empty only for a `header`.
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub value: Option<Value>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub danger: bool,
    #[serde(default)]
    pub confirm: Option<Confirm>,
    /// Asks for a line of text; the action gets `{value, text}`.
    #[serde(default)]
    pub prompt: Option<Prompt>,
    /// Draws a separator above the entry.
    #[serde(default)]
    pub separated: bool,
    /// Shows a checkmark: the entry toggles an option.
    #[serde(default)]
    pub checked: Option<bool>,
    /// Current value in an exclusive choice; painted as a selected row.
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub disabled: bool,
    /// A section label that cannot be picked.
    #[serde(default)]
    pub header: bool,
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

/// Asks for one line of text before the action is sent, such as a new file
/// or branch name.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Prompt {
    pub title: String,
    #[serde(default)]
    pub placeholder: Option<String>,
    /// Initial text, selected for overtyping.
    #[serde(default)]
    pub value: Option<String>,
    /// Label of the confirming button.
    #[serde(default)]
    pub ok: Option<String>,
    /// Selects only the part before the last dot, as a file name without
    /// its extension.
    #[serde(default)]
    pub stem: bool,
}

/// Rejects entries the menu cannot draw or send.
pub fn check_items(items: &[MenuItem]) -> Result<()> {
    for item in items {
        super::widget::known_icon(item.icon.as_deref())?;
        if !item.header && item.action.is_empty() {
            bail!("menu item {:?} needs an action", item.label);
        }
        if item.selected && item.checked.is_some() {
            bail!(
                "menu item {:?} cannot combine selected and checked",
                item.label
            );
        }
        check_ask(item.confirm.as_ref(), item.prompt.as_ref())?;
    }
    Ok(())
}

/// A control either confirms or prompts before its action.
pub fn check_ask(confirm: Option<&Confirm>, prompt: Option<&Prompt>) -> Result<()> {
    if confirm.is_some() && prompt.is_some() {
        bail!("confirm and prompt cannot be combined");
    }
    Ok(())
}
