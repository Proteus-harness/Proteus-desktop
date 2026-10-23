//! The project line above the composer: which folder the chat works in,
//! and a picker that starts a new chat in another one on the same server.
//! A project with saved chats reuses its latest session's settings.

use crate::ui::kit::ButtonLabel as _;
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::chrome::{WorkspaceRequest, folder_name};
use super::fade::faded;
use super::kit;
use super::kit::Pressable as _;
use super::tip::Tip as _;
use super::workspace::Workspace;
use super::{motion, prefs};
use crate::tabs;

/// Width of the picker before zoom.
const PICKER_WIDTH: f32 = 320.;

impl Workspace {
    /// Project of the chat shown by `chat`; the server's folder until the
    /// session list knows it.
    fn chat_project(&self, chat: EntityId) -> PathBuf {
        self.chats
            .iter()
            .find(|(_, open)| open.view.entity_id() == chat)
            .and_then(|(tab, _)| tabs::chat_dir(tab))
            .and_then(|dir| self.sessions.iter().find(|s| s.session_dir == dir))
            .map(|summary| summary.workspace_path.clone())
            .unwrap_or_else(|| self.workspace_path.clone())
    }

    pub(super) fn render_project_chip(&self, chat: EntityId, _: &App) -> AnyElement {
        ProjectChip {
            chat,
            project: self.chat_project(chat),
            picker: self.project_picker.clone(),
        }
        .into_any_element()
    }

    /// Starts a chat in `project`. A blank active chat gives way to it, so
    /// trying folders leaves no empty chats behind.
    pub(super) fn new_chat_in(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = match project.canonicalize() {
            Ok(path) if path.is_dir() => path,
            _ => {
                prefs::update(cx, |prefs| prefs.recent.forget(&project));
                let message = format!("Папка недоступна: {}", project.display());
                return self.report(message, cx);
            }
        };
        prefs::update(cx, |prefs| prefs.recent.touch(&project));
        let blank = self.active_chat_view().and_then(|chat| {
            let dir = self.active_chat_dir()?.to_path_buf();
            chat.read(cx).is_blank(cx).then(|| (chat.entity_id(), dir))
        });
        if let Some((chat, _)) = &blank
            && self.chat_project(*chat) == project
        {
            return self.focus_active_chat(window, cx);
        }
        let blank = blank.map(|(_, dir)| dir);
        if project == self.workspace_path {
            return self.start_chat(None, None, blank, window, cx);
        }
        let source = self
            .sessions
            .iter()
            .filter(|summary| summary.workspace_path == project)
            .max_by_key(|summary| summary.updated_at_ms)
            .map(|summary| summary.session_dir.clone());
        self.start_chat(source, Some(project), blank, window, cx)
    }

    /// Opens a new session with the settings of `source`, or of the server
    /// when there is none, in `workspace` or the source's folder; `replace`
    /// is a blank chat to drop.
    fn start_chat(
        &mut self,
        source: Option<PathBuf>,
        workspace: Option<PathBuf>,
        replace: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_settings(cx);
        self.call_in(
            window,
            cx,
            move |api| {
                if let Some(source) = &source {
                    api.resume(source)?;
                }
                let opened = api.new_session(source.as_deref(), workspace.as_deref())?;
                Ok((opened, api.sessions()?))
            },
            move |this, (opened, sessions), window, cx| {
                this.sessions = sessions;
                this.add_chat(opened.session_dir, None, window, cx);
                if let Some(dir) = replace {
                    this.delete_session(dir, window, cx);
                }
            },
        );
    }

    /// Projects to pick from: the chat's own first, then those with chats
    /// by last change, then recent folders.
    fn project_choices(&self, current: &Path, cx: &App) -> Vec<PathBuf> {
        let mut projects: Vec<(Option<u64>, &Path)> = Vec::new();
        for summary in &self.sessions {
            let path = summary.workspace_path.as_path();
            match projects.iter_mut().find(|(_, known)| *known == path) {
                Some((updated, _)) => *updated = (*updated).max(summary.updated_at_ms),
                None => projects.push((summary.updated_at_ms, path)),
            }
        }
        projects.sort_by_key(|(updated, _)| std::cmp::Reverse(*updated));
        let mut choices = vec![current.to_path_buf()];
        let recent = prefs::current(cx)
            .recent
            .iter()
            .filter(|path| path.is_dir());
        for path in projects.into_iter().map(|(_, path)| path).chain(recent) {
            if !choices.iter().any(|known| known == path) {
                choices.push(path.to_path_buf());
            }
        }
        choices
    }
}

/// Every chat has its own measured trigger; the shared picker belongs to
/// exactly one of them, including while its closing drawing is retained.
#[derive(IntoElement)]
struct ProjectChip {
    chat: EntityId,
    project: PathBuf,
    picker: Entity<ProjectPicker>,
}

impl RenderOnce for ProjectChip {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = SharedString::from(format!("project-chip-{}", self.chat));
        let bounds = window
            .use_keyed_state(id.clone(), cx, |_, _| {
                Rc::new(Cell::new(Bounds::<Pixels>::default()))
            })
            .read(cx)
            .clone();
        let chat = self.chat;
        let open = self.picker.read(cx).popup.is_open()
            && self.picker.read(cx).popup.value().copied() == Some(chat);
        let picker = self.picker.clone();
        let project = self.project.clone();
        let anchor = bounds.clone();
        let trigger = kit::button("project-chip")
            .ghost()
            .xsmall()
            .icon(Lucide::Folder)
            .faded_label(folder_name(&self.project))
            .selected(open)
            .child(motion::menu_chevron(
                SharedString::from(format!("project-chevron-{chat}")),
                open,
                cx.theme().muted_foreground,
            ))
            .text_color(cx.theme().muted_foreground)
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                picker.update(cx, |picker, cx| {
                    if picker.popup.is_open() && picker.popup.value().copied() == Some(chat) {
                        picker.close(window, cx);
                    } else {
                        picker.show(chat, project.clone(), anchor.clone(), window, cx);
                    }
                });
            });
        let frame = self.picker.update(cx, |picker, cx| {
            if picker.popup.value().copied() != Some(chat) {
                return None;
            }
            picker.popup.frame(bounds.get(), window, cx)
        });
        let root = h_flex()
            .id(id.clone())
            .tip_text(self.project.display().to_string())
            .on_prepaint(move |rect, _, _| bounds.set(rect))
            .child(trigger);
        match frame {
            Some(frame) => root.child(motion::popup(self.picker, id, frame)),
            None => root,
        }
    }
}

/// The picker's content: a search over projects and the folder dialog.
pub(super) struct ProjectPicker {
    workspace: WeakEntity<Workspace>,
    search: Entity<InputState>,
    /// Project of the chat the picker was opened from.
    current: PathBuf,
    popup: motion::Popup<EntityId>,
    anchor: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    restore: Option<FocusHandle>,
    _subscription: Subscription,
}

impl ProjectPicker {
    pub(super) fn new(
        workspace: WeakEntity<Workspace>,
        current: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.bind_keys([KeyBinding::new(
            "escape",
            Escape,
            Some("ProteusProjectPicker"),
        )]);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Поиск проектов"));
        let subscription =
            cx.subscribe_in(&search, window, |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { .. } => {
                    if let Some(project) = this.matches(cx).into_iter().next() {
                        this.pick(project, window, cx);
                    }
                }
                _ => {}
            });
        Self {
            workspace,
            search,
            current,
            popup: motion::Popup::default(),
            anchor: Rc::default(),
            focus: cx.focus_handle(),
            restore: None,
            _subscription: subscription,
        }
    }

    fn show(
        &mut self,
        chat: EntityId,
        current: PathBuf,
        anchor: Rc<Cell<Bounds<Pixels>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.focus.contains_focused(window, cx) {
            self.restore = window.focused(cx);
        }
        self.current = current;
        self.anchor = anchor;
        self.popup.show(chat);
        self.search.update(cx, |search, cx| {
            search.set_value("", window, cx);
            search.focus(window, cx);
        });
        cx.notify();
        let _ = self.workspace.update(cx, |_, cx| cx.notify());
    }

    fn matches(&self, cx: &App) -> Vec<PathBuf> {
        let Some(workspace) = self.workspace.upgrade() else {
            return Vec::new();
        };
        let query = self.search.read(cx).value().trim().to_lowercase();
        workspace
            .read(cx)
            .project_choices(&self.current, cx)
            .into_iter()
            .filter(|path| {
                query.is_empty() || path.to_string_lossy().to_lowercase().contains(&query)
            })
            .collect()
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let generation = self.popup.generation();
        if !self.popup.close() {
            return;
        }
        cx.notify();
        let picker = cx.weak_entity();
        // Logical dismissal is immediate; focus/parent notification remains
        // outside this lease. A late close cannot affect a fresh opening.
        window.defer(cx, move |window, cx| {
            let _ = picker.update(cx, |picker, cx| {
                if picker.popup.generation() != generation || picker.popup.is_open() {
                    return;
                }
                motion::restore_focus(&picker.focus, picker.restore.take(), window, cx);
                cx.notify();
                let _ = picker.workspace.update(cx, |_, cx| cx.notify());
            });
        });
    }

    fn pick(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !self.popup.is_open() {
            return;
        }
        self.close(window, cx);
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.new_chat_in(project, window, cx)
            })
            .ok();
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.popup.is_open() {
            return;
        }
        self.close(window, cx);
        self.workspace
            .update(cx, |_, cx| cx.emit(WorkspaceRequest::Choose))
            .ok();
    }
}

impl Render for ProjectPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let matches = self.matches(cx);
        let row = |id: ElementId| {
            h_flex()
                .id(id)
                .px_2()
                .py_1()
                .gap_2()
                .rounded(theme.radius)
                .text_sm()
                .cursor_pointer()
                .pressable(cx)
                .hover(|row| row.bg(theme.list_hover))
        };
        let projects = matches.into_iter().enumerate().map(|(ix, path)| {
            let current = path == self.current;
            row(("project-choice", ix).into())
                .child(
                    Icon::new(Lucide::Folder)
                        .small()
                        .flex_none()
                        .text_color(theme.muted_foreground),
                )
                .child(div().flex_1().min_w_0().child(faded(folder_name(&path))))
                .when(current, |row| {
                    row.child(Icon::new(Lucide::Check).small().flex_none())
                })
                .tip_text(path.display().to_string())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.pick(path.clone(), window, cx)),
                )
        });
        let projects: Vec<_> = projects.collect();
        let empty = projects.is_empty();
        let generation = self.popup.generation();
        let anchor = self.anchor.clone();
        let surface = v_flex()
            .id("project-picker")
            .key_context("ProteusProjectPicker")
            .track_focus(&self.focus)
            .relative()
            .w(prefs::scaled(PICKER_WIDTH))
            .popover_style(cx)
            .p_1()
            .gap_1()
            .when(self.popup.is_open(), |surface| {
                surface
                    .on_action(cx.listener(|this, _: &Escape, window, cx| {
                        this.close(window, cx);
                        cx.stop_propagation();
                    }))
                    .on_mouse_down_out(cx.listener(
                        move |this, event: &MouseDownEvent, window, cx| {
                            if !anchor.get().contains(&event.position)
                                && this.popup.is_current(generation)
                            {
                                this.close(window, cx);
                            }
                        },
                    ))
            })
            .child(
                div()
                    .p_1()
                    .child(super::search::field(&self.search).small()),
            )
            .child(
                v_flex()
                    .id("project-choices")
                    .max_h(prefs::scaled(280.))
                    .overflow_y_scrollbar()
                    .gap_0p5()
                    .children(projects)
                    .when(empty, |list| {
                        list.child(
                            div()
                                .px_2()
                                .py_1()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child("Ничего не найдено"),
                        )
                    }),
            )
            .child(div().h_px().mx_1().bg(theme.border))
            .child(
                row("project-open-folder".into())
                    .child(
                        Icon::new(Lucide::FolderOpen)
                            .small()
                            .flex_none()
                            .text_color(theme.muted_foreground),
                    )
                    .child("Открыть папку…")
                    .on_click(cx.listener(|this, _, window, cx| this.choose(window, cx))),
            );
        if self.popup.is_open() {
            surface
                .focus_trap("project-picker-trap", &self.focus)
                .into_any_element()
        } else {
            surface.into_any_element()
        }
    }
}
