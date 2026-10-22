//! Widget tree an extension process sends; the client renders it natively.
//!
//! The schema is closed: unknown node types or fields are errors, so a
//! process learns about a mistake instead of seeing a silently empty view.

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
    },
    Fill {
        children: Vec<Node>,
    },
    Input {
        id: String,
        value: String,
        placeholder: String,
        action: String,
    },
    Code {
        id: String,
        text: String,
        language: String,
        #[serde(default)]
        height: Option<f32>,
    },
    Terminal {
        id: String,
        cwd: std::path::PathBuf,
    },
    Choice {
        label: String,
        selected: bool,
        action: String,
        value: Value,
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
    },
    List {
        items: Vec<Item>,
    },
    Button {
        label: String,
        action: String,
        #[serde(default)]
        value: Option<Value>,
        #[serde(default)]
        variant: Variant,
        /// Icon name from the manifest icon set, shown before the label.
        #[serde(default)]
        icon: Option<String>,
        /// Marks the active button of a group, such as the current tab.
        #[serde(default)]
        selected: bool,
        /// Hover hint, useful for an icon-only button.
        #[serde(default)]
        hint: Option<String>,
    },
    Badge {
        label: String,
        #[serde(default)]
        tone: Tone,
    },
    Divider {},
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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    #[default]
    Default,
    Muted,
    Accent,
    Success,
    Warning,
    Danger,
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
    #[serde(default)]
    pub children: Vec<Node>,
}

/// A list row; with `action` it is clickable and sends that action.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub value: Option<Value>,
}

fn hundred() -> f64 {
    100.
}

fn twenty() -> usize {
    20
}

impl Node {
    pub fn visit(&self, each: &mut impl FnMut(&Node)) {
        each(self);
        match self {
            Node::Stack { children, .. }
            | Node::Card { children, .. }
            | Node::Fill { children } => {
                for child in children {
                    child.visit(each);
                }
            }
            Node::Tabs { tabs } => {
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
            Node::Code {
                height: Some(height),
                ..
            } if !height.is_finite() || *height <= 0. => {
                bail!("code height must be finite and positive")
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
            | Node::Fill { children } => children,
            Node::Tabs { tabs } => {
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
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(value: Value) -> Result<Node> {
        let node: Node = serde_json::from_value(value)?;
        node.validate()?;
        Ok(node)
    }

    #[test]
    fn reads_a_nested_tree_with_defaults() {
        let node = parse(json!({
            "type": "stack",
            "children": [
                {"type": "metric", "label": "Задачи", "value": "3"},
                {"type": "button", "label": "Обновить", "action": "refresh"},
                {"type": "divider"}
            ]
        }))
        .unwrap();
        let Node::Stack {
            direction,
            gap,
            children,
        } = node
        else {
            panic!("stack expected");
        };
        assert_eq!(
            (direction, gap, children.len()),
            (Direction::Column, Gap::Small, 3)
        );
    }

    #[test]
    fn reads_a_cells_grid() {
        let node = parse(json!({"type": "cells", "segments": [
            {"label": "история", "cells": 3, "color": "#5cc784", "value": "3k"},
            {"label": "свободно", "cells": 17, "tone": "muted"}
        ]}))
        .unwrap();
        let Node::Cells { columns, segments } = node else {
            panic!("cells expected");
        };
        assert_eq!((columns, segments.len()), (20, 2));
        assert_eq!(hex_color("#5cc784"), Some(0x5cc784));
    }

    #[test]
    fn rejects_unknown_types_fields_and_bad_shapes() {
        assert!(parse(json!({"type": "video"})).is_err());
        assert!(parse(json!({"type": "text", "text": "a", "color": "red"})).is_err());
        assert!(parse(json!({"type": "table", "columns": ["a", "b"], "rows": [["1"]]})).is_err());
        assert!(parse(json!({"type": "progress", "value": 1, "max": 0})).is_err());
        assert!(
            parse(json!({"type": "cells", "segments": [
                {"label": "a", "cells": 1, "color": "red"}
            ]}))
            .is_err()
        );
        assert!(parse(json!({"type": "cells", "columns": 0, "segments": []})).is_err());
        let mut deep = json!({"type": "divider"});
        for _ in 0..40 {
            deep = json!({"type": "stack", "children": [deep]});
        }
        assert!(parse(deep).is_err());
    }
}
