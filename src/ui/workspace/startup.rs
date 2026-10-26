//! Only chats on screen belong to the workspace's startup path. Other saved
//! tabs keep their place and resume through the normal open-chat path on use.
use std::path::{Path, PathBuf};

use anyhow::Result;
use proteus_client_common::session_selection::select_startup_session;
use proteus_contracts::app_protocol::AppSessionSummary;

use crate::api::Api;
use crate::connection::{self, BackendGuard};
use crate::launch::LaunchOptions;
use crate::tabs::{self, TabLayout};

#[cfg(test)]
mod tests;

/// Start server and session reads before creating the graphics platform. The
/// receiver owns the result; dropping it also drops/stops an unused backend.
pub(crate) struct Preparation {
    workspace: PathBuf,
    result: async_channel::Receiver<Result<Boot>>,
}

impl Preparation {
    pub(crate) fn launch(launch: &crate::launch::Launch) -> Option<Self> {
        let workspace = launch.workspace.clone()?;
        let options = LaunchOptions {
            workspace: workspace.clone(),
            target: launch.target.clone(),
        };
        let saved = crate::preferences::startup_tabs();
        let (send, result) = async_channel::bounded(1);
        std::thread::Builder::new()
            .name("proteus-startup".into())
            .spawn(move || {
                let _ = send.send_blocking(boot(&options, saved));
            })
            .ok()?;
        Some(Self { workspace, result })
    }

    pub(super) fn for_workspace(
        self,
        workspace: &Path,
    ) -> Option<async_channel::Receiver<Result<Boot>>> {
        (self.workspace == workspace).then_some(self.result)
    }
}

pub(super) struct Boot {
    pub api: Api,
    pub backend: Option<BackendGuard>,
    pub sessions: Vec<AppSessionSummary>,
    pub restored: Vec<PathBuf>,
    pub resumed: Vec<PathBuf>,
    pub startup: Option<PathBuf>,
}

pub(super) fn boot(options: &LaunchOptions, saved: TabLayout) -> Result<Boot> {
    let _trace = crate::startup_trace::span("workspace.boot");
    let (api, backend) = connection::connect(options)?;
    crate::startup_trace::mark("backend.ready");
    restore(api, backend, saved)
}

fn restore(api: Api, backend: Option<BackendGuard>, saved: TabLayout) -> Result<Boot> {
    let mut sessions = Vec::new();
    // Include the command target when the focused tab is a document.
    let active = saved.active_chat(None);
    let resumed: Vec<PathBuf> = saved
        .chats()
        .filter(|id| {
            saved.visible().any(|visible| visible == *id) || active.as_deref() == Some(*id)
        })
        .filter_map(tabs::chat_dir)
        .filter(|dir| {
            api.resume(dir)
                .is_ok_and(|opened| opened.session_dir == *dir)
        })
        .map(Path::to_path_buf)
        .collect();
    let restored = saved
        .chats()
        .filter_map(tabs::chat_dir)
        .map(Path::to_path_buf)
        .collect();
    let startup = if resumed.is_empty() {
        let bootstrap = api.bootstrap()?;
        // A fresh preference file has no restore target. Only that case needs
        // catalog discovery; existing tabs never wait for unrelated journals.
        if bootstrap.session_dir.is_none() {
            sessions = api.sessions()?;
        }
        Some(
            match select_startup_session(None, bootstrap.session_dir, &sessions) {
                Some(dir) => api.resume(Path::new(&dir))?.session_dir,
                None => {
                    let opened = api.new_session(None, None)?.session_dir;
                    sessions = api.sessions()?;
                    opened
                }
            },
        )
    } else {
        None
    };
    Ok(Boot {
        api,
        backend,
        sessions,
        restored,
        resumed,
        startup,
    })
}
