//! A surface of an installed extension: panel, document or icon.

use crate::ui::kit::ButtonLabel as _;
use std::collections::HashMap;

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;

use super::render;
use super::runtime::Runtime;
use crate::declarative::presentation::Presentation;
use crate::declarative::protocol::{Content, Outgoing, SurfaceKind};
use crate::declarative::widget::Node;
use crate::ui::kit;

/// A user gesture can change content height. Runtime notifications and the
/// first asynchronous preview only remeasure; they must not stop following.
pub(super) struct WillResize;

impl EventEmitter<WillResize> for DeclarativeView {}

pub(super) struct DeclarativeView {
    runtime: Entity<Runtime>,
    view: String,
    surface: SurfaceKind,
    root: Option<Node>,
    failure: Option<String>,
    pub(super) images: super::images::Images,
    pub(super) root_revision: u64,
    keys: HashMap<String, String>,
    action_sequence: u64,
    resize_on_reply: bool,
    focus_page: bool,
    pub(super) presentation: Option<Presentation>,
    /// Selected tab of every `tabs` node, by tree path.
    pub(super) tabs: HashMap<String, usize>,
    pub(super) steps: HashMap<String, Entity<super::steps::Stepper>>,
    pub(super) focus: FocusHandle,
    pub(super) fields: super::fields::Fields,
    _runtime: Subscription,
}

impl DeclarativeView {
    pub(super) fn new(
        runtime: Entity<Runtime>,
        surface: SurfaceKind,
        argument: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::with_content(runtime, surface, argument, None, cx)
    }

    pub(super) fn with_content(
        runtime: Entity<Runtime>,
        surface: SurfaceKind,
        argument: Option<String>,
        content: Option<Content>,
        cx: &mut Context<Self>,
    ) -> Self {
        let view = crate::declarative::protocol::view_id(surface, argument.as_deref());
        let me = cx.weak_entity();
        let (id, attach_runtime) = (view.clone(), runtime.clone());
        // Attach after construction: the runtime may answer synchronously.
        cx.defer(move |cx| {
            attach_runtime.update(cx, |runtime, cx| {
                runtime.attach(id, surface, argument, content, me, cx)
            });
        });
        let detach = (runtime.clone(), view.clone());
        cx.on_release(move |_, cx| {
            detach.0.update(cx, |runtime, _| runtime.detach(&detach.1));
        })
        .detach();
        Self {
            _runtime: cx.observe(&runtime, |_, _, cx| cx.notify()),
            runtime,
            view,
            surface,
            root: None,
            failure: None,
            images: super::images::Images::default(),
            root_revision: 0,
            keys: HashMap::new(),
            action_sequence: 0,
            resize_on_reply: false,
            focus_page: false,
            presentation: None,
            tabs: HashMap::new(),
            steps: HashMap::new(),
            focus: cx.focus_handle(),
            fields: super::fields::Fields::default(),
        }
    }

    pub(super) fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }

    pub(super) fn show(
        &mut self,
        root: Node,
        sequence: u64,
        keys: HashMap<String, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if sequence < self.action_sequence {
            return false;
        }
        if std::mem::take(&mut self.resize_on_reply) {
            cx.emit(WillResize);
        }
        self.failure = None;
        self.images.prepare(&root, cx);
        self.keys = keys;
        self.root_revision = self.root_revision.wrapping_add(1);
        if let (
            Some(Node::Stack {
                transition: Some(old),
                ..
            }),
            Node::Stack {
                transition: Some(new),
                ..
            },
        ) = (&self.root, &root)
        {
            self.focus_page |= old != new;
        }
        self.root = Some(root);
        cx.notify();
        true
    }

    pub(super) fn fail(&mut self, error: String, sequence: u64, cx: &mut Context<Self>) {
        if sequence >= self.action_sequence {
            if std::mem::take(&mut self.resize_on_reply) {
                cx.emit(WillResize);
            }
            self.failure = Some(error);
            cx.notify();
        }
    }

    pub(super) fn ready(&self, cx: &App) -> bool {
        let mut unknown = false;
        if let Some(root) = &self.root {
            root.visit(&mut |node| unknown |= matches!(node, Node::Unknown));
        }
        !unknown && self.root.as_ref().is_some_and(|root| {
            !matches!(root, Node::Stack {children, ..} | Node::Fill {children} if children.is_empty())
        }) && self.error(cx).is_none()
    }

    /// Last tree and keys, to seed a reopened popup until the process answers.
    pub(super) fn snapshot(&self) -> Option<(Node, HashMap<String, String>)> {
        Some((self.root.clone()?, self.keys.clone()))
    }

    pub(super) fn present(&mut self, presentation: Option<Presentation>, cx: &mut Context<Self>) {
        self.presentation = presentation;
        cx.notify();
    }

    pub(super) fn act(&mut self, action: String, value: Option<Value>, cx: &mut Context<Self>) {
        self.resize_on_reply = true;
        cx.emit(WillResize);
        self.action_sequence += 1;
        let message = Outgoing::Action {
            sequence: self.action_sequence,
            view: self.view.clone(),
            action,
            value,
        };
        self.runtime.read(cx).send(&message);
    }

    /// Face of the compact icon; `None` shows the extension icon.
    pub(super) fn face(&self, cx: &App) -> Option<(String, Hsla)> {
        self.root
            .as_ref()
            .and_then(|root| render::compact(root, cx))
    }

    pub(super) fn error(&self, cx: &App) -> Option<String> {
        self.failure
            .clone()
            .or_else(|| self.images.error())
            .or_else(|| self.runtime.read(cx).error.clone())
    }

    pub(super) fn popup_closed(&self, cx: &App) -> u64 {
        self.runtime.read(cx).popup_closed
    }

    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        self.root = None;
        self.failure = None;
        self.images = super::images::Images::default();
        self.root_revision = self.root_revision.wrapping_add(1);
        self.keys.clear();
        self.presentation = None;
        self.action_sequence = 0;
        self.resize_on_reply = false;
        // Shells outlive a restarted process: the next tree with the same
        // terminal id picks its PTY up again, and pruning drops the rest.
        let terminals = std::mem::take(&mut self.fields.terminals);
        self.fields = super::fields::Fields::default();
        self.fields.terminals = terminals;
        self.steps.clear();
        self.tabs.clear();
        cx.notify();
    }

    fn render_error(&self, error: String, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        v_flex()
            .gap_2()
            .p_3()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.danger.opacity(0.4))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .child("Расширение не отвечает"),
            )
            .child(
                div()
                    .text_xs()
                    .font_family(theme.mono_font_family.clone())
                    .text_color(theme.muted_foreground)
                    .child(error),
            )
            .child(
                kit::button("declarative-restart")
                    .small()
                    .outline()
                    .faded_label("Перезапустить")
                    .on_click(cx.listener(|this, _, window, cx| {
                        let runtime = this.runtime.clone();
                        window.defer(cx, move |_, cx| {
                            runtime.update(cx, |runtime, cx| runtime.restart(cx));
                        });
                    })),
            )
    }
}

impl Focusable for DeclarativeView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for DeclarativeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::motion::drive(window, cx);
        if std::mem::take(&mut self.focus_page) && self.focus.contains_focused(window, cx) {
            // A root page replaces its controls. Release the old control's
            // focus so the visible page gets the normal first-control focus.
            self.focus.focus(window, cx);
        }
        if let Some(root) = self.root.clone() {
            let scale = window.scale_factor() * crate::ui::prefs::scale(cx);
            if self.images.rasterize(&root, scale, cx) {
                // Propagate a raster error to a content shell before its
                // next layout; ordinary surfaces use the same error banner.
                cx.notify();
            }
            self.prune_fields(&root);
            self.prepare_steps(&root, cx);
            self.prepare_fields(&root, window, cx);
        }
        if self.focus.is_focused(window) {
            // Follow visible tree order, not a HashMap of controls that also
            // contains hidden tabs and collapsed disclosures.
            let focus = self.root.as_ref().and_then(|root| {
                (0..3).find_map(|priority| self.first_control(root, "root", priority, cx))
            });
            if let Some(focus) = focus {
                let container = self.focus.clone();
                cx.defer_in(window, move |_, window, cx| {
                    if container.is_focused(window) {
                        focus.focus(window, cx);
                    }
                });
            }
        }
        let error = self.error(cx);
        let body = match &self.root {
            Some(root) => {
                let root = root.clone();
                Some(self.node(&root, "root", cx))
            }
            None => None,
        };
        let title = self.runtime.read(cx).title();
        let waiting = self.root.is_none() && error.is_none() && self.surface != SurfaceKind::Badge;
        let fill = matches!(self.root, Some(Node::Fill { .. }));
        let view = v_flex()
            .id(ElementId::Name(format!("declarative-{}", self.view).into()))
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !event.keystroke.modifiers.modified()
                    && this
                        .fields
                        .pickers
                        .values()
                        .any(|picker| picker.read(cx).focus(cx).is_focused(window))
                {
                    // The picker owns arrows, Enter and Esc-clear before an
                    // outer popup action is allowed to dismiss it.
                    return;
                }
                let chord = crate::shortcuts::normalize(&event.keystroke.unparse()).ok();
                if chord.as_deref() == Some("ctrl-f")
                    && let Some(focus) = this
                        .root
                        .as_ref()
                        .and_then(|root| this.first_control(root, "root", 3, cx))
                {
                    focus.focus(window, cx);
                    window.refresh();
                    cx.stop_propagation();
                    return;
                }
                if !event.keystroke.modifiers.modified() && event.keystroke.key == "escape" {
                    let mut search_focused = false;
                    if let Some(root) = &this.root {
                        root.visit(&mut |node| {
                            if let Node::Input {
                                id, search: true, ..
                            } = node
                            {
                                search_focused |= this.fields.inputs.get(id).is_some_and(|input| {
                                    input.state.focus_handle(cx).is_focused(window)
                                });
                            }
                        });
                    }
                    if search_focused {
                        // The shared search clears/collapses before an outer
                        // extension or popup Escape action may dismiss it.
                        return;
                    }
                }
                let typing = !event.keystroke.modifiers.modified()
                    && event.keystroke.key != "escape"
                    && this
                        .fields
                        .inputs
                        .values()
                        .any(|input| input.state.focus_handle(cx).is_focused(window));
                if !typing
                    && let Some(action) = chord.and_then(|chord| this.keys.get(&chord)).cloned()
                {
                    this.act(action, None, cx);
                    cx.stop_propagation();
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if this.surface != SurfaceKind::Popup || event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "left" | "right" if this.steps.len() == 1 => {
                        let stepper = this.steps.values().next().unwrap().clone();
                        let offset = if event.keystroke.key == "left" { -1 } else { 1 };
                        cx.defer(move |cx| {
                            stepper.update(cx, |stepper, cx| stepper.step(offset, cx))
                        });
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .w_full()
            .when(
                !matches!(self.surface, SurfaceKind::Popup | SurfaceKind::Content),
                |view| view.h_full(),
            )
            .p_3()
            .gap_3()
            .when(fill, |view| view.p_0().gap_0())
            .when_some(error, |view, error| {
                view.child(self.render_error(error, cx))
            })
            .when_some(body, |view, body| view.child(body))
            .when(waiting, |view| {
                view.child(crate::ui::state::loading(
                    SharedString::from(format!("extension-loading-{}", cx.entity_id())),
                    format!("{title}: загрузка…"),
                    cx,
                ))
            });
        if self.surface == SurfaceKind::Content {
            view.p_0().gap_2().min_w_0().into_any_element()
        } else if fill {
            view.overflow_hidden().into_any_element()
        } else {
            view.overflow_y_scrollbar()
                .id(ElementId::Name(
                    format!("declarative-scroll-{}", self.view).into(),
                ))
                .into_any_element()
        }
    }
}

impl DeclarativeView {
    fn prepare_steps(&mut self, node: &Node, cx: &mut Context<Self>) {
        match node {
            Node::Sortable { items, .. } => {
                for item in items {
                    for child in &item.children {
                        self.prepare_steps(child, cx);
                    }
                }
            }

            Node::Steps {
                id,
                options,
                selected,
                action,
            } => {
                if let Some(stepper) = self.steps.get(id) {
                    stepper.update(cx, |stepper, cx| {
                        if stepper.options != *options || stepper.selected != *selected {
                            stepper.options = options.clone();
                            stepper.selected = *selected;
                            cx.notify();
                        }
                    });
                } else {
                    let (this, action) = (cx.weak_entity(), action.clone());
                    let stepper = cx.new(|_| {
                        super::steps::Stepper::new(options.clone(), *selected, move |index, cx| {
                            let _ = this.update(cx, |this, cx| {
                                this.act(action.clone(), Some(serde_json::json!(index)), cx)
                            });
                        })
                    });
                    self.steps.insert(id.clone(), stepper);
                }
            }
            Node::Stack { children, .. }
            | Node::Card { children, .. }
            | Node::Fill { children }
            | Node::Disclosure { children, .. }
            | Node::Bar { children, .. }
            | Node::PanelEditor { children }
            | Node::Split { children, .. } => {
                for child in children {
                    self.prepare_steps(child, cx);
                }
            }
            Node::Tabs { tabs, .. } => {
                for tab in tabs {
                    for child in &tab.children {
                        self.prepare_steps(child, cx);
                    }
                }
            }
            _ => {}
        }
    }
}

impl DeclarativeView {
    fn first_control(
        &self,
        node: &Node,
        path: &str,
        priority: u8,
        cx: &App,
    ) -> Option<FocusHandle> {
        let focus = match node {
            Node::Picker { id, .. } if priority == 0 => self
                .fields
                .pickers
                .get(id)
                .map(|picker| picker.read(cx).focus(cx)),
            Node::Tree { id, .. } if priority == 1 => {
                self.fields.trees.get(id).map(|tree| tree.read(cx).focus())
            }
            Node::Lanes { id, .. } if priority == 1 => self
                .fields
                .lanes
                .get(id)
                .map(|lanes| lanes.read(cx).focus()),
            Node::List { id, .. } if priority == 1 => {
                self.fields.lists.get(id).map(|list| list.read(cx).focus())
            }
            Node::Diff { id, .. } if priority == 1 => {
                self.fields.diffs.get(id).map(|diff| diff.read(cx).focus())
            }
            Node::Terminal { id, .. } if priority == 1 => self
                .fields
                .terminals
                .get(id)
                .map(|terminal| terminal.read(cx).focus_handle(cx)),
            Node::Code { id, .. } if priority == 1 => self
                .fields
                .codes
                .get(id)
                .map(|code| code.read(cx).focus_handle(cx)),
            Node::Input {
                id, search: true, ..
            } if priority == 3 => self
                .fields
                .inputs
                .get(id)
                .map(|input| input.state.focus_handle(cx)),
            Node::Input {
                id, search: false, ..
            } if priority == 2 => self
                .fields
                .inputs
                .get(id)
                .map(|input| input.state.focus_handle(cx)),
            _ => None,
        };
        if focus.is_some() {
            return focus;
        }
        let children = match node {
            Node::Sortable { items, .. } => {
                return items.iter().find_map(|item| {
                    let path = format!("{path}.sort({}:{})", item.id.len(), item.id);
                    item.children.iter().enumerate().find_map(|(i, child)| {
                        self.first_control(child, &format!("{path}.{i}"), priority, cx)
                    })
                });
            }

            Node::Disclosure {
                id, open, children, ..
            } if self.fields.disclosures.get(id).copied().unwrap_or(*open) => children.as_slice(),
            Node::Stack { children, .. }
            | Node::Fill { children }
            | Node::Card { children, .. }
            | Node::Bar { children, .. }
            | Node::PanelEditor { children }
            | Node::Split { children, .. } => children.as_slice(),
            Node::Tabs { tabs, .. } => {
                let selected = self
                    .tabs
                    .get(path)
                    .copied()
                    .unwrap_or(0)
                    .min(tabs.len().saturating_sub(1));
                return tabs.get(selected).and_then(|tab| {
                    tab.children.iter().enumerate().find_map(|(i, child)| {
                        self.first_control(child, &format!("{path}.{selected}.{i}"), priority, cx)
                    })
                });
            }
            _ => &[],
        };
        children
            .iter()
            .enumerate()
            .find_map(|(i, child)| self.first_control(child, &format!("{path}.{i}"), priority, cx))
    }

    pub(super) fn focus_control(&self, id: &str, window: &mut Window, cx: &mut App) {
        let focus = self
            .fields
            .inputs
            .get(id)
            .map(|input| input.state.focus_handle(cx))
            .or_else(|| {
                self.fields
                    .pickers
                    .get(id)
                    .map(|picker| picker.read(cx).focus(cx))
            })
            .or_else(|| self.fields.trees.get(id).map(|tree| tree.read(cx).focus()))
            .or_else(|| {
                self.fields
                    .lanes
                    .get(id)
                    .map(|lanes| lanes.read(cx).focus())
            })
            .or_else(|| self.fields.lists.get(id).map(|list| list.read(cx).focus()))
            .or_else(|| self.fields.diffs.get(id).map(|diff| diff.read(cx).focus()))
            .or_else(|| {
                self.fields
                    .codes
                    .get(id)
                    .map(|editor| editor.read(cx).focus_handle(cx))
            })
            .or_else(|| {
                self.fields
                    .terminals
                    .get(id)
                    .map(|terminal| terminal.read(cx).focus_handle(cx))
            });
        if let Some(focus) = focus {
            focus.focus(window, cx);
        }
    }
}
