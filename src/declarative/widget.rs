//! Widget tree an extension process sends; the client renders it natively.
//!
//! The schema is closed for fields: a misspelled field is an error, so a
//! process learns about a mistake instead of seeing a silently empty view.
//! A node type from a newer protocol becomes [`Node::Unknown`] and is drawn
//! as a notice; processes check `features` from `hello` before relying on
//! newer nodes.

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 5_000;

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Node {
    /// A square grid split into colored segments, with a legend.
    Cells {
        #[serde(default = "twenty")]
        columns: usize,
        segments: Vec<Segment>,
    },
    Select {
        id: String,
        label: String,
        options: Vec<super::graph::Choice>,
        action: String,
    },
    Graph {
        id: String,
        nodes: Vec<super::graph::Node>,
        edges: Vec<super::graph::Edge>,
        action: String,
    },
    Tree {
        id: String,
        rows: Vec<super::tree::Row>,
        action: String,
        /// Pixels per depth level.
        #[serde(default = "sixteen")]
        indent: f32,
        /// Without chevrons branches show only a folder icon.
        #[serde(default = "yes")]
        chevrons: bool,
        /// Thin lines that join the rows of an open branch.
        #[serde(default = "yes")]
        guides: bool,
    },
    /// A virtualized list with a lane column, such as a commit history.
    Lanes {
        id: String,
        columns: Vec<super::lanes::Column>,
        rows: Vec<super::lanes::Row>,
        action: String,
    },
    Fill {
        children: Vec<Node>,
    },
    /// Native animated disclosure; open is the initial state, remembered by id.
    Disclosure {
        id: String,
        label: String,
        #[serde(default = "yes")]
        open: bool,
        children: Vec<Node>,
    },
    Person {
        name: String,
        detail: String,
    },
    /// A compact strip at an edge of a panel, like the footers of Zed's git
    /// panel: what comes before a `spacer` sits at the start and may shrink,
    /// the rest at the end; buttons and menus in it are compact.
    Bar {
        children: Vec<Node>,
        /// A line above the strip; without it the strip sits right under a
        /// list and is inset like Zed's repository footer.
        #[serde(default = "yes")]
        border: bool,
    },
    /// The editor area at the bottom of a panel, like Zed's
    /// `panel_editor_container`: a line above and the editor background
    /// behind a `flush` input and the bar under it.
    PanelEditor {
        children: Vec<Node>,
    },
    /// A muted message centered in the rest of the room, for an empty list.
    Empty {
        text: String,
    },
    /// Shared delayed spinner; an empty response uses Empty instead.
    Loading {
        text: String,
    },
    /// A small icon, such as the branch icon before a branch menu.
    Icon {
        name: String,
        #[serde(default = "muted")]
        tone: Tone,
    },
    /// Row of full-height columns; `widths[i]` fixes a column width in pixels,
    /// a missing or null entry lets the column take the rest.
    Split {
        children: Vec<Node>,
        #[serde(default)]
        widths: Vec<Option<f32>>,
    },
    Input {
        id: String,
        value: String,
        placeholder: String,
        action: String,
        /// More than one line makes a multi-line field of that height that
        /// scrolls inside.
        #[serde(default = "one")]
        lines: usize,
        /// Sent with the text on Enter, or Ctrl+Enter in a multi-line field.
        #[serde(default)]
        submit: Option<String>,
        /// No frame and no background, across the whole strip, as the
        /// commit editor in a `panel_editor`.
        #[serde(default)]
        flush: bool,
        /// The monospace font of code, like Zed's commit message editor.
        #[serde(default)]
        mono: bool,
        /// Compact inline search: an icon until focused or queried; Ctrl+F
        /// opens it, Esc clears first and then returns focus to the view.
        #[serde(default)]
        search: bool,
    },
    /// Sends `{checked, value}` with the new state.
    Checkbox {
        label: String,
        checked: bool,
        action: String,
        #[serde(default)]
        value: Option<Value>,
    },
    /// A button that opens a menu.
    Menu {
        #[serde(default)]
        label: String,
        #[serde(default)]
        icon: Option<String>,
        #[serde(default)]
        hint: Option<String>,
        items: Vec<super::control::MenuItem>,
    },
    Code {
        id: String,
        text: String,
        language: String,
        #[serde(default)]
        height: Option<f32>,
        /// Soft-wraps long lines and hides line numbers, for prose and logs.
        #[serde(default)]
        wrap: bool,
    },
    /// Self-contained color SVG, rendered as a native cached image.
    Svg {
        source: String,
        width: f32,
        height: f32,
    },
    /// A shared read-only diff, with file cards and responsive split/unified
    /// rows. Optional metadata keeps non-text files visible too.
    Diff {
        id: String,
        patch: String,
        #[serde(default)]
        files: Vec<crate::diff::FileInfo>,
        #[serde(default)]
        height: Option<f32>,
    },
    Terminal {
        id: String,
        cwd: std::path::PathBuf,
    },
    /// Shared whole-row hold-and-drag order; sends {id,before} only on drop.
    Sortable {
        id: String,
        items: Vec<SortableItem>,
        action: String,
    },
    /// Native searchable, keyboard-navigable and virtualized single choice.
    Picker {
        id: String,
        options: Vec<super::graph::Choice>,
        action: String,
        #[serde(default = "search_placeholder")]
        placeholder: String,
        #[serde(default = "no_matches")]
        empty: String,
        /// Short selectors share the picker rows without a search field.
        #[serde(default = "yes")]
        search: bool,
    },
    Steps {
        id: String,
        options: Vec<String>,
        selected: usize,
        action: String,
    },
    Stack {
        #[serde(default)]
        direction: Direction,
        #[serde(default)]
        gap: Gap,
        /// Stable page key; changing it uses the shared page transition.
        #[serde(default)]
        transition: Option<String>,
        #[serde(default)]
        children: Vec<Node>,
    },
    Card {
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        children: Vec<Node>,
    },
    Text {
        text: String,
        /// A label or preview fades at the available width; prose wraps.
        #[serde(default)]
        single_line: bool,
        #[serde(default)]
        tone: Tone,
        #[serde(default)]
        size: TextSize,
        #[serde(default)]
        mono: bool,
    },
    Metric {
        label: String,
        value: String,
        #[serde(default)]
        detail: Option<String>,
        #[serde(default)]
        tone: Tone,
    },
    Progress {
        value: f64,
        #[serde(default = "hundred")]
        max: f64,
        #[serde(default)]
        label: Option<String>,
    },
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    BarChart {
        bars: Vec<Bar>,
        #[serde(default)]
        unit: Option<String>,
    },
    Tabs {
        tabs: Vec<Tab>,
        /// Sent with the index of the tab the user switches to.
        #[serde(default)]
        action: Option<String>,
    },
    /// A virtualized list of two-line rows, such as the commit history of
    /// a panel.
    List {
        id: String,
        rows: Vec<super::list::Row>,
        action: String,
    },
    Button {
        label: String,
        action: String,
        #[serde(default)]
        value: Option<Value>,
        #[serde(default)]
        variant: Variant,
        /// Lucide icon file name, shown before the label.
        #[serde(default)]
        icon: Option<String>,
        /// Marks the active button of a group, such as the current tab.
        #[serde(default)]
        selected: bool,
        /// Hover hint, useful for an icon-only button.
        #[serde(default)]
        hint: Option<String>,
        #[serde(default)]
        confirm: Option<super::control::Confirm>,
        #[serde(default)]
        prompt: Option<super::control::Prompt>,
        /// Adds a chevron part that opens these entries: a split button.
        #[serde(default)]
        menu: Vec<super::control::MenuItem>,
        /// Grayed out; the hint still shows.
        #[serde(default)]
        disabled: bool,
    },
    Badge {
        label: String,
        #[serde(default)]
        tone: Tone,
    },
    Divider {},
    /// Empty room that pushes the following items of a row to its end.
    Spacer {},
    /// A node type this client does not know, from a newer protocol; drawn
    /// as a notice instead of failing the whole view.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub label: String,
    pub cells: usize,
    #[serde(default)]
    pub tone: Tone,
    /// `#rrggbb`; overrides `tone`.
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

const MAX_CELLS: usize = 2_000;

/// Parses `#rrggbb` into `0xrrggbb`.
pub fn hex_color(color: &str) -> Option<u32> {
    let hex = color.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| u32::from_str_radix(hex, 16).ok())
        .flatten()
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    #[default]
    Column,
    Row,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Gap {
    None,
    #[default]
    Small,
    Medium,
    Large,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Default,
    Muted,
    Accent,
    Success,
    Warning,
    Danger,
    /// `#rrggbb` from the process.
    Color(u32),
}

impl<'de> Deserialize<'de> for Tone {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Ok(match name.as_str() {
            "default" => Tone::Default,
            "muted" => Tone::Muted,
            "accent" => Tone::Accent,
            "success" => Tone::Success,
            "warning" => Tone::Warning,
            "danger" => Tone::Danger,
            color => Tone::Color(hex_color(color).ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "tone {color:?} is neither a tone name nor #rrggbb"
                ))
            })?),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TextSize {
    Small,
    #[default]
    Normal,
    Large,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    Primary,
    #[default]
    Default,
    Ghost,
    Danger,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Bar {
    pub label: String,
    pub value: f64,
    #[serde(default)]
    pub tone: Tone,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub label: String,
    /// A small muted «(N)» after the label; zero shows nothing.
    #[serde(default)]
    pub count: Option<usize>,
    #[serde(default)]
    pub children: Vec<Node>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SortableItem {
    pub id: String,
    /// Accessible drag preview; arbitrary native children render the row.
    pub label: String,
    #[serde(default)]
    pub children: Vec<Node>,
}

fn search_placeholder() -> String {
    "Поиск…".into()
}
fn no_matches() -> String {
    "Ничего не найдено".into()
}

fn hundred() -> f64 {
    100.
}

fn one() -> usize {
    1
}

fn twenty() -> usize {
    20
}

fn sixteen() -> f32 {
    16.
}

fn yes() -> bool {
    true
}

fn muted() -> Tone {
    Tone::Muted
}

pub(super) fn known_icon(name: Option<&str>) -> Result<()> {
    match name {
        Some(name) if super::icons::lucide(name).is_none() => {
            bail!("unknown icon {name:?}; use a Lucide icon file name such as \"git-branch\"")
        }
        _ => Ok(()),
    }
}

impl Node {
    pub fn visit(&self, each: &mut impl FnMut(&Node)) {
        each(self);
        match self {
            Node::Stack { children, .. }
            | Node::Card { children, .. }
            | Node::Fill { children }
            | Node::Disclosure { children, .. }
            | Node::Bar { children, .. }
            | Node::PanelEditor { children }
            | Node::Split { children, .. } => {
                for child in children {
                    child.visit(each);
                }
            }
            Node::Sortable { items, .. } => {
                for item in items {
                    for child in &item.children {
                        child.visit(each);
                    }
                }
            }
            Node::Tabs { tabs, .. } => {
                for tab in tabs {
                    for child in &tab.children {
                        child.visit(each);
                    }
                }
            }
            _ => {}
        }
    }
    /// Rejects trees the renderer must not try: too deep or too large, rows
    /// that do not match the columns, non-finite numbers.
    pub fn validate(&self) -> Result<()> {
        let mut count = 0;
        self.check(0, &mut count)
    }

    fn check(&self, depth: usize, count: &mut usize) -> Result<()> {
        *count += 1;
        if depth > MAX_DEPTH {
            bail!("widget tree is deeper than {MAX_DEPTH}");
        }
        if *count > MAX_NODES {
            bail!("widget tree has more than {MAX_NODES} nodes");
        }
        let children: &[Node] = match self {
            Node::Svg {
                source,
                width,
                height,
            } => {
                if !width.is_finite()
                    || !height.is_finite()
                    || !(1. ..=4096.).contains(width)
                    || !(1. ..=4096.).contains(height)
                {
                    bail!("svg dimensions must be finite and within 1..=4096");
                }
                super::svg::validate(source)?;
                &[]
            }
            Node::Code {
                height: Some(height),
                ..
            }
            | Node::Diff {
                height: Some(height),
                ..
            } if !height.is_finite() || *height <= 0. => {
                bail!("code/diff height must be finite and positive")
            }
            Node::Graph { nodes, edges, .. } => {
                let ids: std::collections::HashSet<_> = nodes.iter().map(|node| &node.id).collect();
                if ids.len() != nodes.len()
                    || nodes
                        .iter()
                        .any(|node| !node.x.is_finite() || !node.y.is_finite())
                {
                    bail!("graph requires unique ids and finite positions");
                }
                if edges
                    .iter()
                    .any(|edge| !ids.contains(&edge.from) || !ids.contains(&edge.to))
                {
                    bail!("graph edge references an unknown node");
                }
                &[]
            }
            Node::Sortable { items, .. } => {
                let ids: std::collections::HashSet<_> = items.iter().map(|item| &item.id).collect();
                if ids.len() != items.len() || items.iter().any(|item| item.id.is_empty()) {
                    bail!("sortable requires non-empty unique item ids");
                }
                *count += items.len();
                if *count > MAX_NODES {
                    bail!("sortable has too many items");
                }
                for item in items {
                    for child in &item.children {
                        child.check(depth + 1, count)?;
                    }
                }
                &[]
            }
            Node::Picker { options, .. } => {
                let values: std::collections::HashSet<_> =
                    options.iter().map(|o| o.value.to_string()).collect();
                if options.len() > MAX_NODES
                    || values.len() != options.len()
                    || options.iter().filter(|o| o.selected).count() > 1
                {
                    bail!(
                        "picker requires unique values, at most one selected option and at most {MAX_NODES} options"
                    );
                }
                &[]
            }
            Node::Steps {
                options, selected, ..
            } => {
                if options.is_empty() || *selected >= options.len() {
                    bail!("steps require a selected option");
                }
                &[]
            }
            Node::Stack { children, .. }
            | Node::Card { children, .. }
            | Node::Bar { children, .. }
            | Node::PanelEditor { children }
            | Node::Fill { children } => children,
            Node::Disclosure { children, .. } => children,
            Node::Icon { name, .. } => {
                known_icon(Some(name))?;
                &[]
            }
            Node::Split { children, widths } => {
                if widths.len() > children.len()
                    || widths
                        .iter()
                        .flatten()
                        .any(|width| !width.is_finite() || *width <= 0.)
                {
                    bail!("split widths must be positive and match the columns");
                }
                children
            }
            Node::Tabs { tabs, .. } => {
                for tab in tabs {
                    for child in &tab.children {
                        child.check(depth + 1, count)?;
                    }
                }
                &[]
            }
            Node::Progress { value, max, .. } => {
                if !value.is_finite() || !max.is_finite() || *max <= 0. {
                    bail!("progress needs a finite value and a positive max");
                }
                &[]
            }
            Node::Table { columns, rows } => {
                if let Some(row) = rows.iter().find(|row| row.len() != columns.len()) {
                    bail!(
                        "table row has {} cells, expected {}",
                        row.len(),
                        columns.len()
                    );
                }
                &[]
            }
            Node::Cells { columns, segments } => {
                if !(1..=100).contains(columns) {
                    bail!("cells need 1..=100 columns");
                }
                if segments.iter().map(|segment| segment.cells).sum::<usize>() > MAX_CELLS {
                    bail!("cells grid is larger than {MAX_CELLS}");
                }
                if let Some(color) = segments
                    .iter()
                    .filter_map(|segment| segment.color.as_deref())
                    .find(|color| hex_color(color).is_none())
                {
                    bail!("cell color {color:?} is not #rrggbb");
                }
                &[]
            }
            Node::Button {
                icon,
                confirm,
                prompt,
                menu,
                ..
            } => {
                known_icon(icon.as_deref())?;
                super::control::check_ask(confirm.as_ref(), prompt.as_ref())?;
                super::control::check_items(menu)?;
                &[]
            }
            Node::Menu { icon, items, .. } => {
                known_icon(icon.as_deref())?;
                if items.is_empty() {
                    bail!("menu needs at least one item");
                }
                super::control::check_items(items)?;
                &[]
            }
            Node::Tree { rows, indent, .. } => {
                if !indent.is_finite() || !(0. ..=64.).contains(indent) {
                    bail!("tree indent must be within 0..=64");
                }
                for row in rows {
                    known_icon(row.icon.as_deref())?;
                    super::control::check_items(&row.menu)?;
                }
                &[]
            }
            Node::Lanes { columns, rows, .. } => {
                super::lanes::check(columns, rows)?;
                &[]
            }
            Node::List { rows, .. } => {
                super::list::check(rows)?;
                &[]
            }
            Node::Input {
                search: true,
                lines,
                ..
            } if *lines != 1 => {
                bail!("compact search must be a single-line input")
            }
            Node::Input { lines, .. } if !(1..=40).contains(lines) => {
                bail!("input lines must be within 1..=40")
            }
            Node::BarChart { bars, .. } => {
                if bars
                    .iter()
                    .any(|bar| !bar.value.is_finite() || bar.value < 0.)
                {
                    bail!("bar values must be finite and non-negative");
                }
                &[]
            }
            _ => &[],
        };
        for child in children {
            child.check(depth + 1, count)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
