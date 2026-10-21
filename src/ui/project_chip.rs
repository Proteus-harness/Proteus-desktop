//! The project line above the composer: which folder the chat works in,
//! and a picker that starts a new chat in another one. A project with
//! saved chats opens on the running server; a folder without any restarts
//! the server there when the window started it.

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::popover::{Popover, PopoverState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::chrome::{WorkspaceRequest, folder_name};
use super::fade::faded;
use super::kit;
use super::kit::Pressable as _;
use super::prefs;
use super::tip::Tip as _;
use super::workspace::Workspace;
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

    pub(super) fn render_project_chip(&self, chat: EntityId, cx: &App) -> AnyElement {
        let project = self.chat_project(chat);
        let picker = self.project_picker.clone();
        let focus = picker.read(cx).search.focus_handle(cx);
        let trigger = kit::button("project-chip")
            .ghost()
            .xsmall()
            .icon(Lucide::Folder)
            .label(folder_name(&project))
            .dropdown_caret(true)
            .text_color(cx.theme().muted_foreground);
        let popover = Popover::new("project-chip-popover")
            .anchor(Anchor::BottomLeft)
            .appearance(false)
            .track_focus(&focus)
            .trigger(trigger)
            .on_open_change({
                let picker = picker.clone();
                move |open, window, cx| {
                    if *open {
                        let project = project.clone();
                        picker.update(cx, |picker, cx| picker.reset(project, window, cx));
                    }
                }
            })
            .content(move |_, _, cx: &mut Context<PopoverState>| {
                let popover = cx.entity().downgrade();
                picker.update(cx, |picker, _| picker.popover = Some(popover));
                picker.clone()
            });
        h_flex()
            .tip_text(self.chat_project(chat).display().to_string())
            .child(popover)
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
            return self.start_chat(None, blank, window, cx);
        }
        let source = self
            .sessions
            .iter()
            .filter(|summary| summary.workspace_path == project)
            .max_by_key(|summary| summary.updated_at_ms)
            .map(|summary| summary.session_dir.clone());
        match source {
            Some(source) => self.start_chat(Some(source), blank, window, cx),
            None if self.switchable => cx.emit(WorkspaceRequest::NewChat(project)),
            None => self.report(
                format!(
                    "Сервер работает с папкой {}; другую папку открыть нельзя.",
                    self.workspace_path.display()
                ),
                cx,
            ),
        }
    }

    /// Opens a new session with the folder and settings of `source`, or of
    /// the server when there is none; `replace` is a blank chat to drop.
    fn start_chat(
        &mut self,
        source: Option<PathBuf>,
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
                let opened = api.new_session(source.as_deref())?;
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

/// The picker's content: a search over projects and the folder dialog.
pub(super) struct ProjectPicker {
    workspace: WeakEntity<Workspace>,
    search: Entity<InputState>,
    /// Project of the chat the picker was opened from.
    current: PathBuf,
    /// The open popover, closed once a project is chosen.
    popover: Option<WeakEntity<PopoverState>>,
    _subscription: Subscription,
}

impl ProjectPicker {
    pub(super) fn new(
        workspace: WeakEntity<Workspace>,
        current: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
            popover: None,
            _subscription: subscription,
        }
    }

    fn reset(&mut self, current: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.current = current;
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        cx.notify();
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
        if let Some(popover) = self.popover.take().and_then(|popover| popover.upgrade()) {
            popover.update(cx, |popover, cx| popover.dismiss(window, cx));
        }
    }

    fn pick(&mut self, project: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.new_chat_in(project, window, cx)
            })
            .ok();
    }

    fn choose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        v_flex()
            .w(prefs::scaled(PICKER_WIDTH))
            .popover_style(cx)
            .p_1()
            .gap_1()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    this.close(window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div().p_1().child(
                    Input::new(&self.search)
                        .small()
                        .prefix(Icon::new(IconName::Search).small()),
                ),
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
            )
    }
}
