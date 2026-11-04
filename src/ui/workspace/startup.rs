//! Only displayed chats, their command target and the selected area's owner
//! belong to startup. Inactive chat areas resume lazily on selection.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use proteus_client_common::session_selection::select_startup_session;

use crate::agents::codex::{CodexService, Model, Opened};
use crate::api::Api;
use crate::connection::{self, BackendGuard};
use crate::conversations::{AgentKind, ConversationSummary, from_proteus};
use crate::launch::LaunchOptions;
use crate::tabs::{self, ChatKey, ChatSpaces, TabLayout};

#[cfg(test)]
#[path = "startup/tests.rs"]
mod tests;

/// Start server and session reads before creating the graphics platform. The
/// receiver owns the result; dropping it also drops/stops an unused backend.
pub(crate) struct Preparation {
    workspace: PathBuf,
    result: async_channel::Receiver<Result<Boot>>,
}

impl Preparation {
    pub(crate) fn launch(launch: &crate::launch::Launch, saved: ChatSpaces) -> Option<Self> {
        let workspace = launch.workspace.clone()?;
        let options = LaunchOptions {
            workspace: workspace.clone(),
            target: launch.target.clone(),
            agent: launch.agent,
        };
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
    pub api: Option<Api>,
    pub backend: Option<BackendGuard>,
    pub sessions: Vec<ConversationSummary>,
    pub projects: HashMap<PathBuf, PathBuf>,
    pub restored: Vec<PathBuf>,
    pub resumed: Vec<PathBuf>,
    pub startup: Option<PathBuf>,
    pub streams: HashMap<PathBuf, async_channel::Receiver<connection::PumpItem>>,
    pub codex: Option<CodexService>,
    pub codex_opened: Vec<Opened>,
    pub codex_models: Vec<Model>,
    pub codex_failed: Vec<(String, String)>,
}

/// Override the executable for portable installs and deterministic local peers.
pub(in crate::ui) fn codex_program() -> PathBuf {
    std::env::var_os("PROTEUS_CODEX_EXECUTABLE")
        .filter(|program| !program.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("codex"))
}

#[cfg(test)]
fn required_chats(saved: &TabLayout) -> Vec<ChatKey> {
    required_chats_with_owner(saved, None)
}

/// The area owner supplies extension context even when its tab is hidden or
/// detached. Focusing a child chat must not replace that context after restart.
fn required_chats_with_owner(saved: &TabLayout, owner: Option<&str>) -> Vec<ChatKey> {
    let active = saved.active_chat(None);
    let mut required: Vec<_> = saved
        .chats()
        .filter(|id| {
            saved.visible().any(|visible| visible == *id) || active.as_deref() == Some(*id)
        })
        .filter_map(tabs::chat_key)
        .collect();
    if let Some(owner) = owner.and_then(tabs::chat_key)
        && !required.contains(&owner)
    {
        required.push(owner);
    }
    required
}

fn needs_core(agent: AgentKind, required: &[ChatKey]) -> bool {
    required
        .iter()
        .any(|key| matches!(key, ChatKey::Proteus(_)))
        || (required.is_empty() && agent == AgentKind::Proteus)
}

pub(super) fn boot(options: &LaunchOptions, saved: ChatSpaces) -> Result<Boot> {
    boot_spaces_with(options, saved, connection::connect, |program, cwd| {
        Ok(CodexService::spawn(program, cwd)?)
    })
}

fn boot_spaces_with(
    options: &LaunchOptions,
    saved: ChatSpaces,
    connect: impl FnOnce(&LaunchOptions) -> Result<(Api, Option<BackendGuard>)>,
    spawn_codex: impl FnOnce(PathBuf, PathBuf) -> Result<CodexService>,
) -> Result<Boot> {
    boot_with_owner(
        options,
        saved.active_layout().clone(),
        saved.active_root(),
        connect,
        spawn_codex,
    )
}

#[cfg(test)]
fn boot_with(
    options: &LaunchOptions,
    saved: TabLayout,
    connect: impl FnOnce(&LaunchOptions) -> Result<(Api, Option<BackendGuard>)>,
    spawn_codex: impl FnOnce(PathBuf, PathBuf) -> Result<CodexService>,
) -> Result<Boot> {
    boot_with_owner(options, saved, None, connect, spawn_codex)
}

fn boot_with_owner(
    options: &LaunchOptions,
    saved: TabLayout,
    owner: Option<&str>,
    connect: impl FnOnce(&LaunchOptions) -> Result<(Api, Option<BackendGuard>)>,
    spawn_codex: impl FnOnce(PathBuf, PathBuf) -> Result<CodexService>,
) -> Result<Boot> {
    let _trace = crate::startup_trace::span("workspace.boot");
    let required = required_chats_with_owner(&saved, owner);
    let codex_ids: Vec<_> = required
        .iter()
        .filter_map(|key| match key {
            ChatKey::Codex(id) => Some(id.as_str()),
            ChatKey::Proteus(_) => None,
        })
        .collect();
    let mut boot = if needs_core(options.agent, &required) {
        let (api, backend) = connect(options)?;
        crate::startup_trace::mark("backend.ready");
        restore_core(api, backend, saved, codex_ids.is_empty(), &required)?
    } else {
        Boot {
            api: None,
            backend: None,
            sessions: Vec::new(),
            projects: HashMap::new(),
            restored: saved
                .chats()
                .filter_map(tabs::chat_dir)
                .map(Path::to_path_buf)
                .collect(),
            resumed: Vec::new(),
            startup: None,
            streams: HashMap::new(),
            codex: None,
            codex_opened: Vec::new(),
            codex_models: Vec::new(),
            codex_failed: Vec::new(),
        }
    };
    // Subscribe while graphics is still being created. A ready snapshot can
    // then populate the first chat frame, without a second HTTP subscription.
    if let Some(api) = &boot.api {
        for dir in boot.resumed.iter().chain(boot.startup.iter()) {
            boot.streams.insert(
                dir.clone(),
                connection::spawn_pump(api.clone(), dir.clone()),
            );
        }
    }
    if !codex_ids.is_empty() || (required.is_empty() && options.agent == AgentKind::Codex) {
        let service = spawn_codex(codex_program(), options.workspace.clone())
            .context("Starting Codex app-server")?;
        restore_codex(&mut boot, service, &codex_ids, &options.workspace)?;
        crate::startup_trace::mark("codex.ready");
    }
    Ok(boot)
}

fn restore_codex(boot: &mut Boot, service: CodexService, ids: &[&str], cwd: &Path) -> Result<()> {
    if ids.is_empty() {
        // A new app target owns its own thread. Never pick an arbitrary thread
        // from another Codex client merely because it is the latest one.
        boot.codex_opened.push(service.new_thread(cwd)?);
    } else {
        for id in ids {
            match service.resume(id) {
                Ok(opened) => boot.codex_opened.push(opened),
                // A rejected saved ID belongs to that tab. A broken transport
                // still fails startup, because the remaining replies are unknown.
                Err(crate::agents::codex::Error::Remote(error)) if service.is_connected() => {
                    boot.codex_failed.push((
                        tabs::ChatKey::Codex((*id).to_owned()).tab_id(),
                        format!("Не удалось восстановить чат Codex {id}: {}", error.message),
                    ));
                }
                Err(error) => {
                    return Err(error).with_context(|| format!("Restoring Codex thread {id}"));
                }
            }
        }
    }
    boot.codex_models = service.models().context("Reading Codex model catalog")?;
    boot.codex = Some(service);
    Ok(())
}

#[cfg(test)]
fn restore(api: Api, backend: Option<BackendGuard>, saved: TabLayout) -> Result<Boot> {
    let required = required_chats(&saved);
    restore_core(api, backend, saved, true, &required)
}

fn restore_core(
    api: Api,
    backend: Option<BackendGuard>,
    saved: TabLayout,
    fallback: bool,
    required: &[ChatKey],
) -> Result<Boot> {
    let mut sessions = Vec::new();
    let mut projects = HashMap::new();
    let resumed: Vec<PathBuf> = required
        .iter()
        .filter_map(|key| match key {
            ChatKey::Proteus(dir) => Some(dir.as_path()),
            ChatKey::Codex(_) => None,
        })
        .filter_map(|dir| {
            let opened = api.resume(dir).ok()?;
            if opened.session_dir != dir {
                return None;
            }
            projects.insert(opened.session_dir.clone(), opened.workspace_path);
            Some(opened.session_dir)
        })
        .collect();
    let restored = saved
        .chats()
        .filter_map(tabs::chat_dir)
        .map(Path::to_path_buf)
        .collect();
    let startup = if resumed.is_empty() && fallback {
        let bootstrap = api.bootstrap()?;
        // A fresh preference file has no restore target. Only that case needs
        // catalog discovery; existing tabs never wait for unrelated journals.
        if bootstrap.session_dir.is_none() {
            sessions = api.sessions()?;
        }
        let opened = match select_startup_session(None, bootstrap.session_dir, &sessions) {
            Some(dir) => api.resume(Path::new(&dir))?,
            None => {
                let opened = api.new_session(None, None)?;
                sessions = api.sessions()?;
                opened
            }
        };
        projects.insert(opened.session_dir.clone(), opened.workspace_path);
        Some(opened.session_dir)
    } else {
        None
    };
    Ok(Boot {
        api: Some(api),
        backend,
        sessions: from_proteus(sessions),
        projects,
        restored,
        resumed,
        startup,
        streams: HashMap::new(),
        codex: None,
        codex_opened: Vec::new(),
        codex_models: Vec::new(),
        codex_failed: Vec::new(),
    })
}
