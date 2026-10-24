//! Persistent native control state for process-owned widget trees.

use super::lanes::Lanes;
use super::list::List;
use super::tree::{Layout, Tree};
use super::{terminal::TerminalView, view::DeclarativeView};
use crate::declarative::widget::Node;
use gpui_kit::component::input::{EditorState, InputEvent, InputState, TextareaState};
use gpui_kit::*;
use std::collections::HashMap;

pub(super) enum Text {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl Text {
    pub fn value(&self, cx: &App) -> SharedString {
        match self {
            Text::Line(state) => state.read(cx).value(),
            Text::Area(state) => state.read(cx).value(),
        }
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self {
            Text::Line(state) => state.read(cx).focus_handle(cx),
            Text::Area(state) => state.read(cx).focus_handle(cx),
        }
    }

    fn set_placeholder(&self, placeholder: String, window: &mut Window, cx: &mut App) {
        match self {
            Text::Line(state) => state.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            }),
            Text::Area(state) => state.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            }),
        }
    }

    fn set_value(&self, value: String, window: &mut Window, cx: &mut App) {
        match self {
            Text::Line(state) => state.update(cx, |input, cx| input.set_value(value, window, cx)),
            Text::Area(state) => state.update(cx, |input, cx| input.set_value(value, window, cx)),
        }
    }
}

pub(super) struct TextInput {
    pub state: Text,
    value: String,
    placeholder: String,
    _subscription: Subscription,
}

#[derive(Default)]
pub(super) struct Fields {
    pub disclosures: HashMap<String, bool>,
    pub inputs: HashMap<String, TextInput>,
    pub codes: HashMap<String, Entity<EditorState>>,
    pub terminals: HashMap<String, Entity<TerminalView>>,
    pub graphs: HashMap<String, Entity<super::graph::Graph>>,
    pub trees: HashMap<String, Entity<Tree>>,
    pub lanes: HashMap<String, Entity<Lanes>>,
    pub lists: HashMap<String, Entity<List>>,
    pub diffs: HashMap<String, Entity<crate::ui::diff::DiffView>>,
    pub pickers: HashMap<String, Entity<crate::ui::picker::Picker<serde_json::Value>>>,
}

impl DeclarativeView {
    pub(super) fn prune_fields(&mut self, root: &Node) {
        let mut ids = std::collections::HashSet::new();
        root.visit(&mut |node| match node {
            Node::Input { id, .. } => {
                ids.insert((0, id.clone()));
            }
            Node::Code { id, .. } => {
                ids.insert((1, id.clone()));
            }
            Node::Terminal { id, .. } => {
                ids.insert((2, id.clone()));
            }
            Node::Tree { id, .. } => {
                ids.insert((3, id.clone()));
            }
            Node::Graph { id, .. } => {
                ids.insert((4, id.clone()));
            }
            Node::Steps { id, .. } => {
                ids.insert((5, id.clone()));
            }
            Node::Lanes { id, .. } => {
                ids.insert((6, id.clone()));
            }
            Node::List { id, .. } => {
                ids.insert((7, id.clone()));
            }
            Node::Diff { id, .. } => {
                ids.insert((8, id.clone()));
            }
            Node::Picker { id, .. } => {
                ids.insert((10, id.clone()));
            }
            Node::Disclosure { id, .. } => {
                ids.insert((9, id.clone()));
            }
            _ => {}
        });
        self.fields
            .inputs
            .retain(|id, _| ids.contains(&(0, id.clone())));
        self.fields
            .codes
            .retain(|id, _| ids.contains(&(1, id.clone())));
        self.fields
            .terminals
            .retain(|id, _| ids.contains(&(2, id.clone())));
        self.fields
            .trees
            .retain(|id, _| ids.contains(&(3, id.clone())));
        self.fields
            .graphs
            .retain(|id, _| ids.contains(&(4, id.clone())));
        self.steps.retain(|id, _| ids.contains(&(5, id.clone())));
        self.fields
            .lanes
            .retain(|id, _| ids.contains(&(6, id.clone())));
        self.fields
            .lists
            .retain(|id, _| ids.contains(&(7, id.clone())));
        self.fields
            .diffs
            .retain(|id, _| ids.contains(&(8, id.clone())));
        self.fields
            .pickers
            .retain(|id, _| ids.contains(&(10, id.clone())));
        self.fields
            .disclosures
            .retain(|id, _| ids.contains(&(9, id.clone())));
    }

    pub(super) fn prepare_fields(
        &mut self,
        node: &Node,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match node {
            Node::Sortable { items, .. } => {
                for item in items {
                    for child in &item.children {
                        self.prepare_fields(child, window, cx);
                    }
                }
            }

            Node::Picker {
                id,
                title,
                back_action,
                options,
                action,
                placeholder,
                empty,
                search,
            } => {
                let options = options
                    .iter()
                    .map(|option| crate::ui::picker::Choice {
                        label: option.label.clone().into(),
                        value: option.value.clone(),
                        selected: option.selected,
                    })
                    .collect();
                let (this, action) = (cx.weak_entity(), action.clone());
                let pick = move |value, _: &mut Window, cx: &mut App| {
                    let _ = this.update(cx, |this, cx| this.act(action.clone(), Some(value), cx));
                };
                let back: Option<std::rc::Rc<dyn Fn(&mut Window, &mut App)>> =
                    back_action.as_ref().map(|action| {
                        let (this, action) = (cx.weak_entity(), action.clone());
                        std::rc::Rc::new(move |_: &mut Window, cx: &mut App| {
                            let _ = this.update(cx, |this, cx| this.act(action.clone(), None, cx));
                        }) as std::rc::Rc<dyn Fn(&mut Window, &mut App)>
                    });
                if let Some(picker) = self.fields.pickers.get(id) {
                    picker.update(cx, |picker, cx| {
                        picker.set_header(title.clone().map(Into::into), back, cx);
                        picker.set_pick(pick);
                        picker.set_options(
                            options,
                            placeholder.clone().into(),
                            empty.clone().into(),
                            window,
                            cx,
                        );
                        picker.set_searchable(*search, window, cx);
                    });
                } else {
                    let picker = cx.new(|cx| {
                        let mut picker = crate::ui::picker::Picker::new(
                            options,
                            placeholder.clone().into(),
                            empty.clone().into(),
                            window,
                            cx,
                            pick,
                        );
                        picker.set_header(title.clone().map(Into::into), back, cx);
                        picker.set_searchable(*search, window, cx);
                        picker
                    });
                    self.fields.pickers.insert(id.clone(), picker);
                }
            }
            Node::Disclosure {
                id, open, children, ..
            } => {
                self.fields.disclosures.entry(id.clone()).or_insert(*open);
                for child in children {
                    self.prepare_fields(child, window, cx);
                }
            }
            Node::Diff {
                id, patch, files, ..
            } => {
                if let Some(diff) = self.fields.diffs.get(id) {
                    diff.update(cx, |diff, cx| diff.set_patch(patch, files, cx));
                } else {
                    let diff = cx
                        .new(|cx| crate::ui::diff::DiffView::new(patch.clone(), files.clone(), cx));
                    self.fields.diffs.insert(id.clone(), diff);
                }
            }
            Node::Graph {
                id,
                nodes,
                edges,
                action,
            } => {
                if let Some(graph) = self.fields.graphs.get(id) {
                    graph.update(cx, |graph, cx| {
                        graph.set_data(nodes.clone(), edges.clone(), cx)
                    });
                } else {
                    let (this, action) = (cx.weak_entity(), action.clone());
                    let graph = cx.new(|_| {
                        super::graph::Graph::new(nodes.clone(), edges.clone(), move |id, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.act(action.clone(), Some(serde_json::json!(id)), cx)
                            });
                        })
                    });
                    self.fields.graphs.insert(id.clone(), graph);
                }
            }
            Node::Input {
                id,
                value,
                placeholder,
                action,
                lines,
                submit,
                ..
            } => {
                if let Some(input) = self.fields.inputs.get_mut(id) {
                    if input.placeholder != *placeholder {
                        input.placeholder = placeholder.clone();
                        input.state.set_placeholder(placeholder.clone(), window, cx);
                    }
                    if input.value != *value {
                        input.value = value.clone();
                        if input.state.value(cx).as_ref() != value {
                            input.state.set_value(value.clone(), window, cx);
                        }
                    }
                } else {
                    let multi = *lines > 1;
                    let (action, submit, identity) = (action.clone(), submit.clone(), id.clone());
                    let handle =
                        move |this: &mut DeclarativeView,
                              text: SharedString,
                              event: &InputEvent,
                              cx: &mut Context<DeclarativeView>| {
                            match event {
                                InputEvent::Change
                                    if this
                                        .fields
                                        .inputs
                                        .get(&identity)
                                        .is_some_and(|input| input.value != text.as_ref()) =>
                                {
                                    this.act(
                                        action.clone(),
                                        Some(serde_json::json!(text.to_string())),
                                        cx,
                                    )
                                }
                                InputEvent::PressEnter { secondary, .. } if *secondary == multi => {
                                    if let Some(submit) = &submit {
                                        this.act(
                                            submit.clone(),
                                            Some(serde_json::json!(text.to_string())),
                                            cx,
                                        )
                                    }
                                }
                                _ => {}
                            }
                        };
                    let (state, subscription) = if multi {
                        let state = cx.new(|cx| {
                            let mut input = TextareaState::new(window, cx)
                                .auto_grow(*lines, *lines)
                                .placeholder(placeholder.clone());
                            input.set_value(value.clone(), window, cx);
                            input
                        });
                        let subscription = cx.subscribe(&state, move |this, state, event, cx| {
                            let text = state.read(cx).value();
                            handle(this, text, event, cx)
                        });
                        (Text::Area(state), subscription)
                    } else {
                        let state = cx.new(|cx| {
                            let mut input =
                                InputState::new(window, cx).placeholder(placeholder.clone());
                            input.set_value(value.clone(), window, cx);
                            input
                        });
                        let subscription = cx.subscribe(&state, move |this, state, event, cx| {
                            let text = state.read(cx).value();
                            handle(this, text, event, cx)
                        });
                        (Text::Line(state), subscription)
                    };
                    self.fields.inputs.insert(
                        id.clone(),
                        TextInput {
                            state,
                            value: value.clone(),
                            placeholder: placeholder.clone(),
                            _subscription: subscription,
                        },
                    );
                }
            }
            Node::Code {
                id,
                text,
                language,
                wrap,
                ..
            } => {
                let editor = self.fields.codes.entry(id.clone()).or_insert_with(|| {
                    cx.new(|cx| {
                        let mut editor = EditorState::new(window, cx)
                            .language(language.clone())
                            .line_number(!*wrap)
                            .soft_wrap(*wrap);
                        editor.set_readonly(true, cx);
                        editor
                    })
                });
                editor.update(cx, |editor, cx| {
                    editor.set_highlighter(language.clone(), cx);
                    if editor.value().as_ref() != text {
                        editor.set_value(text.clone(), window, cx);
                    }
                });
            }
            Node::Terminal { id, cwd } => {
                if !self.fields.terminals.contains_key(id) {
                    let terminal = cx.new(|cx| TerminalView::new(cwd.clone(), window, cx));
                    let focus = self.focus.clone();
                    let target = terminal.read(cx).focus_handle(cx);
                    cx.defer_in(window, move |_, window, cx| {
                        if focus.is_focused(window) {
                            target.focus(window, cx);
                        }
                    });
                    self.fields.terminals.insert(id.clone(), terminal);
                }
            }
            Node::Tree {
                id,
                rows,
                action,
                indent,
                chevrons,
                guides,
            } => {
                let layout = Layout {
                    indent: *indent,
                    chevrons: *chevrons,
                    guides: *guides,
                };
                if let Some(tree) = self.fields.trees.get(id) {
                    tree.update(cx, |tree, cx| tree.set_rows(rows.clone(), layout, cx));
                } else {
                    let send = self.row_sender(action, cx);
                    let view = cx.weak_entity();
                    let tree = cx.new(|cx| Tree::new(rows.clone(), layout, view, cx, send));
                    self.fields.trees.insert(id.clone(), tree);
                }
            }
            Node::Lanes {
                id,
                columns,
                rows,
                action,
            } => {
                if let Some(lanes) = self.fields.lanes.get(id) {
                    lanes.update(cx, |lanes, cx| {
                        lanes.set_columns(columns, cx);
                        lanes.set_rows(rows.clone(), cx);
                    });
                } else {
                    let send = self.row_sender(action, cx);
                    let view = cx.weak_entity();
                    let lanes = cx.new(|cx| {
                        let mut lanes = Lanes::new(rows.clone(), view, cx, send);
                        lanes.set_columns(columns, cx);
                        lanes
                    });
                    self.fields.lanes.insert(id.clone(), lanes);
                }
            }
            Node::List { id, rows, action } => {
                if let Some(list) = self.fields.lists.get(id) {
                    list.update(cx, |list, cx| list.set_rows(rows.clone(), cx));
                } else {
                    let send = self.row_sender(action, cx);
                    let view = cx.weak_entity();
                    let list = cx.new(|cx| List::new(rows.clone(), view, cx, send));
                    self.fields.lists.insert(id.clone(), list);
                }
            }
            Node::Stack { children, .. }
            | Node::Card { children, .. }
            | Node::Fill { children }
            | Node::Bar { children, .. }
            | Node::PanelEditor { children }
            | Node::Split { children, .. } => {
                for child in children {
                    self.prepare_fields(child, window, cx);
                }
            }
            Node::Tabs { tabs, .. } => {
                for tab in tabs {
                    for child in &tab.children {
                        self.prepare_fields(child, window, cx);
                    }
                }
            }
            _ => {}
        }
    }
}

impl DeclarativeView {
    /// Sends `{id, event}` of a tree, lanes or list row with `action`.
    fn row_sender(
        &self,
        action: &str,
        cx: &mut Context<Self>,
    ) -> impl Fn(String, &'static str, &mut App) + 'static {
        let (this, action) = (cx.weak_entity(), action.to_string());
        move |id, event, cx| {
            let _ = this.update(cx, |this, cx| {
                this.act(
                    action.clone(),
                    Some(serde_json::json!({"id": id, "event": event})),
                    cx,
                )
            });
        }
    }
}
