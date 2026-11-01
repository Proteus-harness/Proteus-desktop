//! Project navigation used by the sessions sidebar and folder launcher.
//! A new chat inherits the latest session settings for its chosen project.

use super::{prefs, workspace::Workspace};
use crate::conversations::{AgentKind, ConversationSummary};
use crate::tabs::{self, ChatKey};
use gpui_kit::*;
use std::path::{Path, PathBuf};

impl Workspace {
    /// Document and panel extensions follow the selected conversation's
    /// folder; the backend/profile launch folder remains immutable.
    pub(super) fn project_for_key(&self, key: Option<&ChatKey>) -> &Path {
        key.and_then(ChatKey::proteus_dir)
            .and_then(|dir| self.session_projects.get(dir))
            .map(PathBuf::as_path)
            .unwrap_or_else(|| session_project(&self.sessions, key, &self.workspace_path))
    }

    pub(super) fn active_project_path(&self) -> &Path {
        self.project_for_key(self.active_chat_key().as_ref())
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
        let agent = prefs::current(cx).preferred_agent;
        if agent == AgentKind::Codex {
            if let Some(key) = latest_main_session(&self.sessions, &project, agent) {
                return self.open_chat(key, project_open(), window, cx);
            }
            let listed_project = project.clone();
            return self.call_codex_in(
                window,
                cx,
                move |service| Ok(service.list(&listed_project, None)?),
                move |this, page, window, cx| {
                    for summary in page.data {
                        let summary = crate::conversations::from_codex(&summary);
                        this.sessions.retain(|row| row.key != summary.key);
                        this.sessions.push(summary);
                    }
                    if let Some(key) =
                        latest_main_session(&this.sessions, &project, AgentKind::Codex)
                    {
                        this.open_chat(key, project_open(), window, cx);
                    } else {
                        this.new_codex(project, None, window, cx);
                    }
                },
            );
        }
        self.call_in(
            window,
            cx,
            |api| api.sessions(),
            move |this, sessions, window, cx| {
                this.set_sessions(sessions, cx);
                if let Some(key) = latest_main_session(&this.sessions, &project, AgentKind::Proteus)
                {
                    this.open_chat(key, project_open(), window, cx);
                } else {
                    this.new_chat_in(project, window, cx);
                }
            },
        );
    }

    /// Project of the chat shown by `chat`; the server's folder until the
    /// session list knows it.
    fn chat_project(&self, chat: EntityId) -> PathBuf {
        let key = self
            .chats
            .iter()
            .find(|(_, open)| open.view.entity_id() == chat)
            .and_then(|(tab, _)| tabs::chat_key(tab));
        self.project_for_key(key.as_ref()).to_path_buf()
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
        let agent = prefs::current(cx).preferred_agent;
        let blank = self.active_chat_view().and_then(|chat| {
            let key = self.active_chat_key()?;
            (AgentKind::for_key(&key) == agent && chat.read(cx).is_blank(cx))
                .then(|| (chat.entity_id(), key))
        });
        if let Some((chat, _)) = &blank
            && self.chat_project(*chat) == project
        {
            return self.focus_active_chat(window, cx);
        }
        if agent == AgentKind::Codex {
            return self.new_codex(project, None, window, cx);
        }
        let blank = blank.and_then(|(_, key)| key.proteus_dir().map(Path::to_path_buf));
        if project == self.workspace_path {
            return self.start_chat(None, None, blank, window, cx);
        }
        let source = self
            .sessions
            .iter()
            .filter(|summary| {
                summary.workspace_path == project
                    && summary.parent.is_none()
                    && matches!(summary.key, ChatKey::Proteus(_))
            })
            .max_by_key(|summary| summary.updated_at_ms)
            .and_then(|summary| summary.key.proteus_dir().map(Path::to_path_buf));
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
                    this.delete_session(ChatKey::Proteus(dir), window, cx);
                }
            },
        );
    }
}

fn project_open() -> super::extensions::OpenDocument {
    super::extensions::OpenDocument {
        preview: false,
        focus: true,
    }
}

fn session_project<'a>(
    sessions: &'a [ConversationSummary],
    key: Option<&ChatKey>,
    fallback: &'a Path,
) -> &'a Path {
    key.and_then(|key| sessions.iter().find(|summary| &summary.key == key))
        .map(|summary| summary.workspace_path.as_path())
        .unwrap_or(fallback)
}

/// Folder navigation chooses a main conversation belonging to the requested
/// driver, never a newer implementation subagent or another driver's chat.
fn latest_main_session(
    sessions: &[ConversationSummary],
    project: &Path,
    agent: AgentKind,
) -> Option<ChatKey> {
    sessions
        .iter()
        .filter(|summary| {
            summary.workspace_path == project
                && summary.parent.is_none()
                && AgentKind::for_key(&summary.key) == agent
        })
        .max_by_key(|summary| summary.updated_at_ms)
        .map(|summary| summary.key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(
        key: ChatKey,
        project: &str,
        updated: u64,
        parent: Option<ChatKey>,
    ) -> ConversationSummary {
        ConversationSummary {
            key,
            parent,
            workspace_path: project.into(),
            updated_at_ms: Some(updated),
            message_count: 1,
            preview: None,
            activity: None,
        }
    }

    #[::core::prelude::v1::test]
    fn project_context_uses_each_drivers_child_folder_and_safe_fallback() {
        let core = ChatKey::Proteus("/child".into());
        let codex = ChatKey::Codex("opaque:child".into());
        let sessions = [
            summary(
                core.clone(),
                "/core-worktree",
                1,
                Some(ChatKey::Proteus("/main".into())),
            ),
            summary(
                codex.clone(),
                "/codex-worktree",
                1,
                Some(ChatKey::Codex("opaque:main".into())),
            ),
        ];
        let fallback = Path::new("/launch");
        assert_eq!(
            session_project(&sessions, Some(&core), fallback),
            Path::new("/core-worktree")
        );
        assert_eq!(
            session_project(&sessions, Some(&codex), fallback),
            Path::new("/codex-worktree")
        );
        assert_eq!(
            session_project(&sessions, Some(&ChatKey::Codex("unknown".into())), fallback),
            fallback
        );
        assert_eq!(session_project(&sessions, None, fallback), fallback);
    }

    #[::core::prelude::v1::test]
    fn folder_lands_in_latest_main_chat_of_selected_driver_instead_of_newer_child() {
        let core = ChatKey::Proteus("/core".into());
        let codex = ChatKey::Codex("opaque:main".into());
        let sessions = [
            summary(core.clone(), "/project", 10, None),
            summary(ChatKey::Codex("opaque:old".into()), "/project", 5, None),
            summary(codex.clone(), "/project", 20, None),
            summary(
                ChatKey::Codex("opaque:child".into()),
                "/project",
                30,
                Some(codex.clone()),
            ),
            summary(
                ChatKey::Codex("opaque:other".into()),
                "/elsewhere",
                40,
                None,
            ),
        ];
        assert_eq!(
            latest_main_session(&sessions, Path::new("/project"), AgentKind::Proteus),
            Some(core)
        );
        assert_eq!(
            latest_main_session(&sessions, Path::new("/project"), AgentKind::Codex),
            Some(codex)
        );
        assert_eq!(
            latest_main_session(&sessions, Path::new("/missing"), AgentKind::Codex),
            None
        );
    }
}
