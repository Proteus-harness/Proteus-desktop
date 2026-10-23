//! Rows of a list with a lane column, such as a commit history: every row
//! puts its dot on a lane, and segments join lanes across row edges.

use super::control::MenuItem;
use super::widget::Tone;
use anyhow::{Result, bail};
use serde::Deserialize;

/// Lanes a row may use.
pub const LANES: usize = 64;
/// Distinct line colors.
pub const COLORS: usize = 8;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub id: String,
    /// Lane of the row's dot.
    pub lane: usize,
    /// Color of the dot.
    pub color: usize,
    /// Lines from the top edge to the middle of the row.
    #[serde(default)]
    pub up: Vec<Segment>,
    /// Lines from the middle to the bottom edge of the row.
    #[serde(default)]
    pub down: Vec<Segment>,
    pub label: String,
    /// Chips after the label, such as branch names.
    #[serde(default)]
    pub refs: Vec<Ref>,
    #[serde(default)]
    pub detail: Option<String>,
    /// Right-aligned muted text.
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub selected: bool,
    /// Opened on right click.
    #[serde(default)]
    pub menu: Vec<MenuItem>,
    #[serde(default)]
    pub hint: Option<String>,
}

/// A line from lane `from` at its start edge to lane `to` at its end.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub from: usize,
    pub to: usize,
    pub color: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ref {
    pub label: String,
    #[serde(default)]
    pub tone: Tone,
}

impl Row {
    /// The widest lane this row draws on.
    pub fn width(&self) -> usize {
        self.up
            .iter()
            .chain(&self.down)
            .flat_map(|segment| [segment.from, segment.to])
            .fold(self.lane, usize::max)
            + 1
    }
}

pub fn check(rows: &[Row]) -> Result<()> {
    for row in rows {
        let segments = row.up.iter().chain(&row.down);
        if row.lane >= LANES
            || segments
                .clone()
                .any(|segment| segment.from >= LANES || segment.to >= LANES)
        {
            bail!("lanes must be below {LANES}");
        }
        if row.color >= COLORS || segments.clone().any(|segment| segment.color >= COLORS) {
            bail!("lane colors must be below {COLORS}");
        }
        super::control::check_items(&row.menu)?;
    }
    Ok(())
}
