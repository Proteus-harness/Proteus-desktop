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

pub(super) struct Boot {
    pub api: Api,
    pub backend: Option<BackendGuard>,
    pub sessions: Vec<AppSessionSummary>,
    pub restored: Vec<PathBuf>,
    pub resumed: Vec<PathBuf>,
    pub startup: Option<PathBuf>,
}

pub(super) fn boot(options: &LaunchOptions, saved: TabLayout) -> Result<Boot> {
    let (api, backend) = connection::connect(options)?;
    restore(api, backend, saved)
}

fn restore(api: Api, backend: Option<BackendGuard>, saved: TabLayout) -> Result<Boot> {
    let mut sessions = api.sessions()?;
    let mut saved = saved;
    saved.retain(|id| {
        tabs::chat_dir(id)
            .is_none_or(|dir| sessions.iter().any(|summary| summary.session_dir == dir))
    });
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
