//! Project navigation used by the sessions sidebar and folder launcher.
//! A new chat inherits the latest session settings for its chosen project.

use super::{prefs, workspace::Workspace};
use crate::tabs;
use gpui_kit::*;
use std::path::PathBuf;

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
}
