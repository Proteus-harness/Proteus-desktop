//! Project navigation used by the sessions sidebar and folder launcher.
//! A new chat inherits the latest session settings for its chosen project.

use super::{prefs, workspace::Workspace};
use crate::tabs;
use gpui_kit::*;
use std::path::{Path, PathBuf};

impl Workspace {
    /// Document and panel extensions follow the selected conversation's
    /// folder; the backend/profile launch folder remains immutable.
    pub(super) fn project_for_session(&self, dir: Option<&Path>) -> &Path {
        dir.and_then(|dir| self.session_projects.get(dir))
            .map(PathBuf::as_path)
            .unwrap_or_else(|| session_project(&self.sessions, dir, &self.workspace_path))
    }

    pub(super) fn active_project_path(&self) -> &Path {
        self.project_for_session(self.active_chat_dir())
    }

    /// Open a project's latest main conversation without replacing the
    /// shared backend or disturbing running chats in other projects.
    pub(super) fn open_project(
        &mut self,
        project: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = match project.canonicalize() {
            Ok(path) if path.is_dir() => path,
            _ => {
                prefs::update(cx, |prefs| prefs.recent.forget(&project));
                return self.report(format!("Папка недоступна: {}", project.display()), cx);
            }
        };
        prefs::update(cx, |prefs| prefs.recent.touch(&project));
        self.session_list.collapsed.remove(&project);
        self.call_in(
            window,
            cx,
            |api| api.sessions(),
            move |this, sessions, window, cx| {
                let latest = latest_main_session(&sessions, &project);
                this.set_sessions(sessions, cx);
                if let Some(dir) = latest {
                    this.open_chat(
                        dir,
                        super::extensions::OpenDocument {
                            preview: false,
                            focus: true,
                        },
                        window,
                        cx,
                    );
                } else {
                    this.new_chat_in(project, window, cx);
                }
            },
        );
    }

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
            .filter(|summary| {
                summary.workspace_path == project && summary.parent_session_id.is_none()
            })
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
                this.session_projects
                    .insert(opened.session_dir.clone(), opened.workspace_path);
                this.set_sessions(sessions, cx);
                this.add_chat(opened.session_dir, None, window, cx);
                if let Some(dir) = replace {
                    this.delete_session(dir, window, cx);
                }
            },
        );
    }
}

fn session_project<'a>(
    sessions: &'a [proteus_contracts::app_protocol::AppSessionSummary],
    dir: Option<&Path>,
    fallback: &'a Path,
) -> &'a Path {
    dir.and_then(|dir| sessions.iter().find(|summary| summary.session_dir == dir))
        .map(|summary| summary.workspace_path.as_path())
        .unwrap_or(fallback)
}

/// Explicit folder navigation never selects an implementation subagent as
/// the project's landing conversation, even if it changed more recently.
fn latest_main_session(
    sessions: &[proteus_contracts::app_protocol::AppSessionSummary],
    project: &std::path::Path,
) -> Option<PathBuf> {
    sessions
        .iter()
        .filter(|summary| summary.workspace_path == project && summary.parent_session_id.is_none())
        .max_by_key(|summary| summary.updated_at_ms)
        .map(|summary| summary.session_dir.clone())
}

#[cfg(test)]
mod tests {
    use super::{latest_main_session, session_project};
    use std::path::{Path, PathBuf};

    #[test]
    fn project_context_uses_the_selected_childs_own_folder_and_safe_fallback() {
        let sessions = [
            ("/main", "/project", None),
            (
                "/child",
                "/worktree",
                Some("00000000-0000-0000-0000-000000000001"),
            ),
        ]
        .into_iter()
        .map(|(dir, project, parent)| {
            serde_json::from_value(serde_json::json!({
                "session_dir": dir, "session_id": "00000000-0000-0000-0000-000000000002",
                "workspace_path": project, "message_count": 1, "preview": null, "updated_at_ms": 1,
                "parent_session_id": parent,
            }))
            .unwrap()
        })
        .collect::<Vec<_>>();
        let fallback = Path::new("/launch");
        assert_eq!(
            session_project(&sessions, Some(Path::new("/child")), fallback),
            Path::new("/worktree")
        );
        assert_eq!(
            session_project(&sessions, Some(Path::new("/main")), fallback),
            Path::new("/project")
        );
        assert_eq!(
            session_project(&sessions, Some(Path::new("/unlisted")), fallback),
            fallback
        );
        assert_eq!(session_project(&sessions, None, fallback), fallback);
    }

    #[test]
    fn folder_lands_in_latest_main_chat_instead_of_latest_child() {
        let sessions = [("/a", "/project", 10, false), ("/child", "/project", 30, true),
            ("/b", "/project", 20, false), ("/other", "/elsewhere", 40, false)]
            .into_iter().map(|(dir, project, updated, child)| serde_json::from_value(serde_json::json!({
                "session_dir": dir, "session_id": "00000000-0000-0000-0000-000000000001",
                "workspace_path": project, "message_count": 1, "preview": null, "updated_at_ms": updated,
                "parent_session_id": child.then_some("00000000-0000-0000-0000-000000000002"),
            })).unwrap()).collect::<Vec<_>>();
        assert_eq!(
            latest_main_session(&sessions, Path::new("/project")),
            Some(PathBuf::from("/b"))
        );
        assert_eq!(latest_main_session(&sessions, Path::new("/missing")), None);
    }
}
