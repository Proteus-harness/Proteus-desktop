// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (TerminalToolHeader) and crates/terminal/src/terminal.rs (new_display_only).
//! Native viewer of one exact tool-owned process; it never owns that process.
pub(in crate::ui) mod cells;
mod io;
mod pending;
#[cfg(test)]
mod tests;
mod view;

use crate::ui::{extensions::SurfaceView, workspace::Workspace};
use crate::{
    api::Api,
    terminal::{Display, Grid},
};
use gpui_kit::*;
use proteus_contracts::contracts::ToolTerminalRef;
use serde::{Deserialize, Serialize};
use std::{cell::Cell, path::PathBuf, rc::Rc};

const PREFIX: &str = "agent-terminal:";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::ui) struct Target {
    pub owner: String,
    pub session_dir: PathBuf,
    pub terminal: ToolTerminalRef,
}
impl Target {
    fn id(&self) -> String {
        format!(
            "{PREFIX}{}",
            serde_json::to_string(self).expect("terminal target")
        )
    }
    pub fn parse(id: &str) -> Option<Self> {
        let target: Self = serde_json::from_str(id.strip_prefix(PREFIX)?).ok()?;
        (crate::tabs::chat_key(&target.owner).is_some() && target.session_dir.is_absolute())
            .then_some(target)
    }
}

pub(super) struct Remote {
    target: Target,
    api: Api,
    workspace: WeakEntity<Workspace>,
    focus: FocusHandle,
    display: Option<Display>,
    area: Rc<Cell<Option<Size<Pixels>>>>,
    grid: Grid,
    remote_grid: Option<Grid>,
    resize_inflight: bool,
    cursor: Option<u64>,
    pending: Option<pending::Pending>,
    reading: bool,
    scheduled: bool,
    finished: bool,
    process_finished: bool,
    input_blocked: bool,
    resume_input: bool,
    exit_code: Option<i32>,
    gap: bool,
    pipe_cr: bool,
    tty: bool,
    stopping: bool,
    error: Option<String>,
    notice: Option<String>,
    scroll_rest: f32,
    _tasks: Vec<Task<()>>,
    _intercept: Subscription,
}

impl Workspace {
    pub(in crate::ui) fn open_agent_terminal(
        &mut self,
        owner: String,
        session_dir: PathBuf,
        terminal: ToolTerminalRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.spaces.active_root() != Some(owner.as_str()) || !self.begin_navigation(cx) {
            return;
        }
        let target = Target {
            owner,
            session_dir,
            terminal,
        };
        let id = target.id();
        if self.show_external_tab(&id, cx) {
            return;
        }
        // Opening a freshly bound tool may replace an obsolete connection's viewer.
        if let Some(surface) = self.documents.get(&id)
            && let Ok(view) = surface.view.clone().downcast::<Remote>()
            && self
                .api()
                .is_none_or(|api| api.connection_id() != view.read(cx).api.connection_id())
        {
            self.documents.remove(&id);
        }
        if self.ensure_agent_terminal(&id, window, cx).is_none() {
            return;
        }
        let parent = self
            .layout
            .pane_of(&target.owner)
            .unwrap_or(self.layout.focused);
        let below = self.layout.neighbor(parent, crate::tabs::Edge::Bottom);
        if self.layout.contains(&id) || below.is_some() {
            self.show_tab_in(&id, below, window, cx);
        } else {
            self.update_tabs(cx, |layout| {
                layout.open_in(&id, parent, |_| false);
                layout.keep(&id);
                layout.split(&id, parent, crate::tabs::Edge::Bottom);
            });
            self.focus_active(window, cx);
        }
    }

    pub(in crate::ui) fn ensure_agent_terminal(
        &mut self,
        id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<SurfaceView> {
        let target = Target::parse(id)?;
        if self.spaces.active_root() != Some(target.owner.as_str()) {
            return None;
        }
        if let Some(surface) = self.documents.get(id) {
            return Some(surface.clone());
        }
        let api = self.api()?;
        let workspace = cx.weak_entity();
        let view = cx.new(|cx| Remote::new(target, api, workspace, cx));
        let focus = view.read(cx).focus.clone();
        let moved = view.downgrade();
        let surface = SurfaceView {
            view: view.into(),
            focus,
            rehost: Rc::new(move |workspace, _, cx| {
                moved
                    .update(cx, |view, cx| {
                        view.workspace = workspace;
                        cx.notify();
                    })
                    .ok();
            }),
        };
        self.documents.insert(id.to_owned(), surface.clone());
        Some(surface)
    }
}

impl Remote {
    fn current(&self, entity: EntityId, cx: &App) -> bool {
        self.workspace.upgrade().is_some_and(|workspace| {
            let workspace = workspace.read(cx);
            workspace
                .api()
                .is_some_and(|api| api.connection_id() == self.api.connection_id())
                && workspace.document_current(&self.target.id(), entity)
        })
    }
    fn visible(&self, entity: EntityId, cx: &App) -> bool {
        self.workspace.upgrade().is_some_and(|workspace| {
            let workspace = workspace.read(cx);
            workspace
                .api()
                .is_some_and(|api| api.connection_id() == self.api.connection_id())
                && workspace.document_visible(&self.target.id(), entity)
        })
    }
}
