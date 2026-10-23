//! Rows of a `list`: two lines each, like the commit history of Zed's git
//! panel. The first line is the label with chips and a mark, the second a
//! small muted line of facts.

use super::control::MenuItem;
use anyhow::Result;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    pub label: String,
    /// Small framed labels after the label, such as tags; a few are shown
    /// and a «+k» chip stands for the rest.
    #[serde(default)]
    pub chips: Vec<String>,
    /// Lucide icon in a small frame at the end of the first line, such as
    /// an arrow on a commit that is not pushed yet.
    #[serde(default)]
    pub mark: Option<String>,
    /// A round person mark at the start of the second line.
    #[serde(default)]
    pub avatar: bool,
    /// Parts of the second line, joined by dots; the first one gives way
    /// when the row is narrow, empty parts are left out.
    #[serde(default)]
    pub meta: Vec<String>,
    /// Hover hint: a title and a muted line under it.
    #[serde(default)]
    pub hint: Option<String>,
    #[serde(default)]
    pub hint_detail: Option<String>,
    /// Opened on right click.
    #[serde(default)]
    pub menu: Vec<MenuItem>,
}

pub fn check(rows: &[Row]) -> Result<()> {
    for row in rows {
        super::widget::known_icon(row.mark.as_deref())?;
        super::control::check_items(&row.menu)?;
    }
    Ok(())
}
