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
fn reads_a_split_with_fixed_and_flexible_columns() {
    let node = parse(json!({"type": "split", "widths": [240, null], "children": [
        {"type": "text", "text": "слева"},
        {"type": "fill", "children": [{"type": "text", "text": "справа"}]}
    ]}))
    .unwrap();
    let Node::Split { children, widths } = node else {
        panic!("split expected");
    };
    assert_eq!((children.len(), widths), (2, vec![Some(240.), None]));
    let one = json!({"type": "text", "text": "x"});
    for widths in [json!([0]), json!([10, 10])] {
        assert!(parse(json!({"type": "split", "widths": widths, "children": [one]})).is_err());
    }
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
    assert!(
        parse(json!({"type": "button", "label": "", "action": "a", "icon": "refresh"})).is_err()
    );
    assert!(parse(json!({"type": "menu", "items": []})).is_err());
    assert!(parse(json!({"type": "text", "text": "a", "tone": "#12345"})).is_err());
    assert!(parse(json!({"type": "input", "id": "i", "value": "", "placeholder": "", "action": "a", "lines": 0})).is_err());
}

#[test]
fn unknown_node_types_become_placeholders() {
    let node = parse(json!({"type": "stack", "children": [
        {"type": "video", "src": "x.mp4", "loop": true}
    ]}))
    .unwrap();
    let Node::Stack { children, .. } = node else {
        panic!("stack expected")
    };
    assert_eq!(children, vec![Node::Unknown]);
}

#[test]
fn reads_controls_icons_and_colors() {
    let node = parse(json!({"type": "stack", "children": [
        {"type": "checkbox", "label": "Amend", "checked": true, "action": "amend"},
        {"type": "menu", "icon": "git-branch", "items": [
            {"label": "Discard", "action": "discard", "danger": true, "separated": true,
             "confirm": {"title": "Sure?", "danger": true}}
        ]},
        {"type": "input", "id": "m", "value": "", "placeholder": "", "action": "m", "lines": 4, "submit": "commit"},
        {"type": "button", "label": "", "action": "r", "icon": "refresh-cw", "confirm": {"title": "Again?"}},
        {"type": "text", "text": "x", "tone": "#E5C07B"},
        {"type": "tree", "id": "t", "action": "t", "rows": [
            {"id": "a", "label": "a.rs", "icon": "file-code", "icon_tone": "#00ff00", "checked": false,
             "strike": true, "menu": [{"label": "Copy", "action": "copy", "icon": "copy"}]}
        ]}
    ]}))
    .unwrap();
    let Node::Stack { children, .. } = node else {
        panic!("stack expected")
    };
    assert!(matches!(
        &children[4],
        Node::Text {
            tone: Tone::Color(0xe5c07b),
            ..
        }
    ));
    assert!(matches!(
        &children[2],
        Node::Input {
            lines: 4,
            submit: Some(_),
            ..
        }
    ));
    assert!(
        parse(json!({"type": "tree", "id": "t", "action": "t", "rows": [
            {"id": "a", "label": "a", "menu": [{"label": "x", "action": "x", "icon": "nope"}]}
        ]}))
        .is_err()
    );
}

#[test]
fn reads_tree_layout_options_and_rejects_bad_indents() {
    let node = parse(json!({"type": "tree", "id": "t", "action": "t", "rows": []})).unwrap();
    assert!(matches!(
        node,
        Node::Tree {
            indent: 16.,
            chevrons: true,
            guides: true,
            ..
        }
    ));
    let node = parse(json!({"type": "tree", "id": "t", "action": "t", "rows": [],
        "indent": 20, "chevrons": false, "guides": false}))
    .unwrap();
    assert!(matches!(
        node,
        Node::Tree {
            chevrons: false,
            guides: false,
            ..
        }
    ));
    assert!(
        parse(json!({"type": "tree", "id": "t", "action": "t", "rows": [], "indent": 100}))
            .is_err()
    );
}

#[test]
fn reads_menu_options_prompts_and_split_buttons() {
    let node = parse(
        json!({"type": "button", "label": "Commit", "action": "commit",
        "disabled": true, "hint": "Nothing staged", "menu": [
            {"label": "Mode", "header": true},
            {"label": "Amend", "action": "amend", "checked": false},
            {"label": "Push", "action": "push", "disabled": true},
            {"label": "Rename", "action": "rename", "value": "a.rs",
             "prompt": {"title": "New name", "value": "a.rs", "stem": true, "ok": "Rename"}}
        ]}),
    )
    .unwrap();
    let Node::Button { menu, disabled, .. } = node else {
        panic!("button expected")
    };
    assert!(disabled && menu[0].header && menu[1].checked == Some(false));
    assert!(menu[3].prompt.as_ref().is_some_and(|prompt| prompt.stem));
    assert!(
        parse(json!({"type": "button", "label": "x", "action": "x",
            "prompt": {"title": "Name"}}))
        .is_ok()
    );
    for bad in [
        json!({"type": "menu", "items": [{"label": "No action"}]}),
        json!({"type": "menu", "items": [{"label": "x", "action": "x",
            "confirm": {"title": "a"}, "prompt": {"title": "b"}}]}),
        json!({"type": "menu", "items": [{"label": "x", "action": "x",
            "prompt": {"title": "b", "select": "all"}}]}),
        json!({"type": "button", "label": "x", "action": "x", "menu": [{"label": "y"}]}),
    ] {
        assert!(parse(bad).is_err());
    }
}

#[test]
fn reads_lanes_and_checks_their_indexes() {
    let row = |lane: usize, color: usize| {
        json!({"id": "c", "lane": lane, "color": color, "label": "Fix",
            "up": [{"from": 0, "to": lane, "color": color}],
            "down": [{"from": lane, "to": 1, "color": 2}],
            "refs": [{"label": "main", "tone": "accent"}],
            "detail": "Ann", "time": "2 h", "selected": true, "hint": "abc123",
            "menu": [{"label": "Copy", "action": "copy"}]})
    };
    let node =
        parse(json!({"type": "lanes", "id": "log", "action": "log", "rows": [row(1, 7)]})).unwrap();
    let Node::Lanes { rows, .. } = node else {
        panic!("lanes expected")
    };
    assert_eq!(rows[0].width(), 2);
    for (lane, color) in [(64, 0), (0, 8)] {
        assert!(
            parse(json!({"type": "lanes", "id": "l", "action": "l", "rows": [row(lane, color)]}))
                .is_err()
        );
    }
}

#[test]
fn reads_spacers_and_tab_actions() {
    let node = parse(json!({"type": "stack", "direction": "row", "children": [
        {"type": "text", "text": "main"}, {"type": "spacer"},
        {"type": "tabs", "action": "tab", "tabs": [{"label": "Changes 3"}]}
    ]}))
    .unwrap();
    let Node::Stack { children, .. } = node else {
        panic!("stack expected")
    };
    assert_eq!(children[1], Node::Spacer {});
    assert!(matches!(
        &children[2],
        Node::Tabs {
            action: Some(_),
            ..
        }
    ));
    assert!(parse(json!({"type": "spacer", "size": 4})).is_err());
}

#[test]
fn reads_panel_strips_lists_and_tab_counts() {
    let node = parse(json!({"type": "fill", "children": [
        {"type": "tabs", "tabs": [{"label": "Изменения", "count": 3, "children": [
            {"type": "empty", "text": "Нет изменений"}
        ]}, {"label": "История", "children": [
            {"type": "list", "id": "log", "action": "log", "rows": [
                {"id": "c1", "label": "Fix", "chips": ["v1", "v2", "v3", "v4"], "mark": "arrow-up",
                 "avatar": true, "meta": ["Ann", "2 ч", "abc1234"], "hint": "Открыть коммит",
                 "hint_detail": "abc1234", "menu": [{"label": "Copy", "action": "copy"}]}
            ]}
        ]}]},
        {"type": "bar", "border": false, "children": [
            {"type": "icon", "name": "git-branch"},
            {"type": "menu", "label": "main", "items": [{"label": "main", "action": "checkout"}]},
            {"type": "spacer"},
            {"type": "button", "label": "Получить", "action": "fetch"}
        ]},
        {"type": "panel_editor", "children": [
            {"type": "input", "id": "m", "value": "", "placeholder": "", "action": "m",
             "lines": 6, "flush": true, "mono": true},
            {"type": "bar", "children": [{"type": "spacer"}]}
        ]}
    ]}))
    .unwrap();
    let Node::Fill { children } = node else {
        panic!("fill expected")
    };
    let Node::Tabs { tabs, .. } = &children[0] else {
        panic!("tabs expected")
    };
    assert_eq!((tabs[0].count, tabs[1].count), (Some(3), None));
    let Node::List { rows, .. } = &tabs[1].children[0] else {
        panic!("list expected")
    };
    assert_eq!((rows[0].chips.len(), rows[0].avatar), (4, true));
    assert!(matches!(&children[1], Node::Bar { border: false, .. }));
    assert!(matches!(
        &children[2],
        Node::PanelEditor { children } if matches!(children[1], Node::Bar { border: true, .. })
    ));
    for bad in [
        json!({"type": "icon", "name": "nope"}),
        json!({"type": "list", "id": "l", "action": "l", "rows": [{"id": "a", "label": "a", "mark": "nope"}]}),
        json!({"type": "list", "id": "l", "action": "l", "rows": [{"id": "a", "label": "a", "time": "1"}]}),
        json!({"type": "bar", "children": [], "gap": 2}),
    ] {
        assert!(parse(bad).is_err());
    }
}
