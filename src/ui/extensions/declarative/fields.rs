//! Persistent native control state for process-owned widget trees.

use super::lanes::Lanes;
use super::list::List;
use super::tree::{Layout, Tree};
use super::{terminal::TerminalView, view::DeclarativeView};
use crate::declarative::widget::Node;
use gpui_kit::component::input::{EditorState, InputEvent, InputState, TextareaState};
use gpui_kit::*;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) mod textarea;

#[cfg(test)]
mod tests;

/// A retained widget keeps its event sink, but its current declaration names the action.
pub(super) struct WidgetAction {
    name: String,
    send: Box<dyn Fn(String, serde_json::Value, &mut App)>,
}

impl WidgetAction {
    pub(super) fn new(
        name: &str,
        send: impl Fn(String, serde_json::Value, &mut App) + 'static,
    ) -> Self {
        Self {
            name: name.to_owned(),
            send: Box::new(send),
        }
    }

    fn rebind(&mut self, name: &str) {
        if self.name != name {
            name.clone_into(&mut self.name);
        }
    }

    pub(super) fn emit(&self, value: serde_json::Value, cx: &mut App) {
        (self.send)(self.name.clone(), value, cx);
    }

    pub(super) fn row(&self, id: &str, event: &'static str, cx: &mut App) {
        self.emit(serde_json::json!({"id": id, "event": event}), cx);
    }
}

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
    pub settings: bool,
    pub grow: bool,
    preview_open: bool,
    // Bindings may change while the native field with this id stays mounted.
    action: String,
    submit: Option<String>,
    lines: usize,
    value: String,
    placeholder: String,
    _subscription: Subscription,
}

impl TextInput {
    fn action_for(&self, text: &str, event: &InputEvent) -> Option<&str> {
        match event {
            InputEvent::Change if self.value != text => Some(&self.action),
            InputEvent::PressEnter { secondary, .. }
                if *secondary == matches!(self.state, Text::Area(_)) =>
            {
                self.submit.as_deref()
            }
            _ => None,
        }
    }
}

/// Embedded prose keeps only its source and disclosure state, without an
/// editor, highlighter or private scroll handle. Replacing the variant drops
/// the previous kind immediately, including same-id language changes. Editor
/// embedding mode changes also replace the entity: folding, wrap and scrollbar
/// policies belong to that mode, and stale callbacks cannot cross it.
pub(super) enum Code {
    Editor {
        state: Entity<EditorState>,
        embedded: bool,
        horizontal_overflow: bool,
    },
    Plain(super::plain_code::Plain),
}

impl Code {
    pub(super) fn editor(&self) -> Option<&Entity<EditorState>> {
        match self {
            Self::Editor { state, .. } => Some(state),
            Self::Plain(_) => None,
        }
    }

    pub(super) fn horizontal_overflow(&self) -> bool {
        matches!(
            self,
            Self::Editor {
                horizontal_overflow: true,
                ..
            }
        )
    }
}

#[derive(Default)]
pub(super) struct Fields {
    pub rich_text: HashMap<String, super::rich_text::RichText>,
    pub contents: HashMap<String, super::rich_text::Content>,
    pub disclosures: HashMap<String, bool>,
    pub inputs: HashMap<String, TextInput>,
    pub codes: HashMap<String, Code>,
    pub terminals: HashMap<String, Entity<TerminalView>>,
    pub graphs: HashMap<String, Entity<super::graph::Graph>>,
    pub trees: HashMap<String, Entity<Tree>>,
    pub lanes: HashMap<String, Entity<Lanes>>,
    pub lists: HashMap<String, Entity<List>>,
    pub diffs: HashMap<String, Entity<crate::ui::diff::DiffView>>,
    pub pickers: HashMap<String, Entity<crate::ui::picker::Picker<serde_json::Value>>>,
}

impl Fields {
    fn set_input_preview(&mut self, id: &str, open: bool) {
        if let Some(input) = self.inputs.get_mut(id) {
            input.preview_open = open;
        }
    }
}

impl DeclarativeView {
    pub(super) fn prune_fields(&mut self, root: &Node) {
        let mut ids = std::collections::HashSet::new();
        root.visit(&mut |node| match node {
            Node::RichText { id, .. } => {
                ids.insert((11, id.clone()));
            }
            Node::Content { id, .. } => {
                ids.insert((12, id.clone()));
            }
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
            .rich_text
            .retain(|id, _| ids.contains(&(11, id.clone())));
        self.fields
            .contents
            .retain(|id, _| ids.contains(&(12, id.clone())));
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
        self.prepare_fields_in(node, self.settings_page(), window, cx);
    }

    fn prepare_fields_in(
        &mut self,
        node: &Node,
        settings: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match node {
            Node::SettingRow { control, .. } => self.prepare_fields_in(control, true, window, cx),
            Node::PanelEditor { children } => {
                for child in children {
                    self.prepare_fields_in(child, false, window, cx);
                }
            }
            Node::RichText { id, html, inlines } => {
                self.prepare_rich_text(id, html, inlines, window, cx)
            }
            Node::Content {
                id,
                language,
                source,
            } => self.prepare_content(id, language, source, window, cx),
            Node::Sortable { items, .. } => {
                for item in items {
                    for child in &item.children {
                        self.prepare_fields_in(child, settings, window, cx);
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
                        group: option.group.clone().map(Into::into),
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
                    self.prepare_fields_in(child, settings, window, cx);
                }
            }
            Node::Diff {
                id, patch, files, ..
            } => {
                if let Some(diff) = self.fields.diffs.get(id) {
                    diff.update(cx, |diff, cx| diff.set_document_patch(patch, files, cx));
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
                        graph.action.rebind(action);
                        graph.set_data(nodes.clone(), edges.clone(), cx)
                    });
                } else {
                    let send = self.widget_action(action, cx);
                    let graph =
                        cx.new(|_| super::graph::Graph::new(nodes.clone(), edges.clone(), send));
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
                let grow = settings && *lines > 1;
                if let Some(input) = self.fields.inputs.get_mut(id) {
                    let layout_changed =
                        input.grow != grow || input.lines != *lines || input.value != *value;
                    if input.action != *action {
                        input.action.clone_from(action);
                    }
                    if input.submit != *submit {
                        input.submit.clone_from(submit);
                    }
                    input.lines = *lines;
                    input.settings = settings;
                    input.grow = grow;
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
                    if layout_changed && let Text::Area(state) = &input.state {
                        textarea::resize(state, *lines, grow, cx);
                    }
                } else {
                    let multi = *lines > 1;
                    let identity = id.clone();
                    let handle =
                        move |this: &mut DeclarativeView,
                              text: SharedString,
                              event: &InputEvent,
                              cx: &mut Context<DeclarativeView>| {
                            let Some(input) = this.fields.inputs.get(&identity) else {
                                return;
                            };
                            let action = input.action_for(&text, event);
                            if let Some(action) = action {
                                this.act(
                                    action.to_owned(),
                                    Some(serde_json::json!(text.to_string())),
                                    cx,
                                );
                            }
                        };
                    let (state, subscription) = if multi {
                        let state = cx.new(|cx| {
                            let mut input = TextareaState::new(window, cx)
                                .auto_grow(*lines, textarea::max_rows(value, *lines, grow))
                                .placeholder(placeholder.clone());
                            input.set_value(value.clone(), window, cx);
                            input
                        });
                        let input_id = id.clone();
                        let subscription = cx.subscribe(&state, move |this, state, event, cx| {
                            if matches!(event, InputEvent::Change)
                                && let Some(input) = this.fields.inputs.get(&input_id)
                            {
                                let (min_rows, grow) = (input.lines, input.grow);
                                textarea::resize(&state, min_rows, grow, cx);
                                if grow && textarea::preview(&state.read(cx).value()).is_some() {
                                    this.fields.set_input_preview(&input_id, true);
                                }
                            }
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
                            settings,
                            grow,
                            preview_open: false,
                            action: action.clone(),
                            submit: submit.clone(),
                            lines: *lines,
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
                embedded,
                ..
            } => {
                if *embedded && super::code_block::plain_language(language) {
                    match self.fields.codes.get_mut(id) {
                        Some(Code::Plain(plain)) => plain.set_source(text),
                        _ => {
                            self.fields.codes.insert(
                                id.clone(),
                                Code::Plain(super::plain_code::Plain::new(text)),
                            );
                        }
                    }
                    return;
                }
                let line_numbers = super::code_block::line_numbers(language, *wrap);
                let editor = match self.fields.codes.get(id) {
                    Some(Code::Editor {
                        state,
                        embedded: mode,
                        ..
                    }) if mode == embedded => state.clone(),
                    _ => {
                        let editor = cx.new(|cx| {
                            let mut editor = EditorState::new(window, cx)
                                .language(language.clone())
                                .rows(1)
                                .line_number(line_numbers)
                                .min_line_number_digits(if *embedded { 1 } else { 3 })
                                .show_trailing_empty_line(!*embedded)
                                .folding(!*embedded)
                                .soft_wrap(*wrap && !*embedded)
                                .scroll_beyond_last_line(if *embedded { Some(0) } else { None });
                            editor.set_readonly(true, cx);
                            editor
                        });
                        self.fields.codes.insert(
                            id.clone(),
                            Code::Editor {
                                state: editor.clone(),
                                embedded: *embedded,
                                horizontal_overflow: false,
                            },
                        );
                        editor
                    }
                };
                let scrollbar_renderer = embedded.then(|| {
                    super::code_geometry::embedded_editor_scrollbar(
                        cx.weak_entity(),
                        id.clone(),
                        editor.entity_id(),
                    )
                });
                editor.update(cx, |editor, cx| {
                    editor.set_scrollbar_renderer(scrollbar_renderer);
                    editor.set_highlighter(language.clone(), cx);
                    editor.set_line_number(line_numbers, window, cx);
                    editor.set_min_line_number_digits(if *embedded { 1 } else { 3 }, cx);
                    editor.set_show_trailing_empty_line(!*embedded, cx);
                    editor.set_scroll_beyond_last_line(
                        if *embedded { Some(0) } else { None },
                        window,
                        cx,
                    );
                    if *embedded {
                        editor.set_scroll_bounce(false, cx);
                    }
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
                    tree.update(cx, |tree, cx| {
                        tree.action.rebind(action);
                        tree.set_rows(Arc::clone(rows), layout, cx);
                    });
                } else {
                    let send = self.widget_action(action, cx);
                    let view = cx.weak_entity();
                    let natural = self.embedded;
                    let tree =
                        cx.new(|cx| Tree::new(Arc::clone(rows), layout, natural, view, cx, send));
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
                        lanes.action.rebind(action);
                        lanes.set_columns(columns, cx);
                        lanes.set_rows(rows, cx);
                    });
                } else {
                    let send = self.widget_action(action, cx);
                    let view = cx.weak_entity();
                    let lanes = cx.new(|cx| {
                        let mut lanes = Lanes::new(rows, view, cx, send);
                        lanes.set_columns(columns, cx);
                        lanes
                    });
                    self.fields.lanes.insert(id.clone(), lanes);
                }
            }
            Node::List { id, rows, action } => {
                if let Some(list) = self.fields.lists.get(id) {
                    list.update(cx, |list, cx| {
                        list.action.rebind(action);
                        list.set_rows(rows, cx);
                    });
                } else {
                    let send = self.widget_action(action, cx);
                    let view = cx.weak_entity();
                    let list = cx.new(|cx| List::new(rows, view, cx, send));
                    self.fields.lists.insert(id.clone(), list);
                }
            }
            Node::Stack { children, .. }
            | Node::Card { children, .. }
            | Node::Fill { children }
            | Node::Bar { children, .. }
            | Node::Split { children, .. } => {
                for child in children {
                    self.prepare_fields_in(child, settings, window, cx);
                }
            }
            Node::Tabs { tabs, .. } => {
                for tab in tabs {
                    for child in &tab.children {
                        self.prepare_fields_in(child, settings, window, cx);
                    }
                }
            }
            _ => {}
        }
    }
}

impl DeclarativeView {
    fn widget_action(&self, action: &str, cx: &mut Context<Self>) -> WidgetAction {
        let this = cx.weak_entity();
        WidgetAction::new(action, move |name, value, cx| {
            let _ = this.update(cx, |this, cx| this.act(name, Some(value), cx));
        })
    }
}
