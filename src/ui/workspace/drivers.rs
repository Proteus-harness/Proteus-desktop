//! Blocking driver creation runs outside GPUI. Detached windows share leases.
use std::sync::{Arc, Mutex};

use anyhow::Result;
use gpui_kit::*;

use super::Workspace;
use crate::agents::codex::CodexService;
use crate::api::Api;
use crate::connection::{self, BackendGuard};
use crate::conversations::AgentKind;
use crate::launch::LaunchOptions;

pub(super) struct CoreConnection {
    pub api: Api,
    pub backend: Option<BackendGuard>,
}

pub(super) fn core(
    slot: &Arc<Mutex<Option<CoreConnection>>>,
    options: &LaunchOptions,
) -> Result<(Api, Option<async_channel::Receiver<String>>)> {
    let mut slot = slot.lock().unwrap();
    if let Some(core) = slot.as_ref() {
        return Ok((core.api.clone(), None));
    }
    let (api, backend) = connection::connect(options)?;
    let exited = backend.as_ref().map(|guard| guard.exited.clone());
    *slot = Some(CoreConnection {
        api: api.clone(),
        backend,
    });
    Ok((api, exited))
}

impl Workspace {
    pub(in crate::ui) fn core_context(&self) -> bool {
        !matches!(self.space_root_key(), Some(crate::tabs::ChatKey::Codex(_)))
    }

    pub(super) fn fail_openings(&mut self, agent: AgentKind, cx: &mut Context<Self>) {
        let failed: Vec<_> = self
            .opening
            .iter()
            .filter(|(id, _)| {
                crate::tabs::chat_key(id).is_some_and(|key| AgentKind::for_key(&key) == agent)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in failed {
            self.opening.remove(&id);
            self.fail_chat_resume(&id, "Подключение к агенту недоступно".into(), cx);
        }
        let deleting = self.deleting_sessions.len();
        self.deleting_sessions
            .retain(|key| AgentKind::for_key(key) != agent);
        if deleting != self.deleting_sessions.len() {
            self.session_catalog_epoch = self.session_catalog_epoch.wrapping_add(1);
        }
    }

    pub(super) fn core_options(&self) -> LaunchOptions {
        LaunchOptions {
            workspace: self.workspace_path.clone(),
            target: self.target.clone(),
            agent: AgentKind::Proteus,
        }
    }

    pub(in crate::ui) fn call_codex_in<R: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        job: impl FnOnce(&CodexService) -> Result<R> + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let slot = self.windows.codex_slot();
        let cwd = self.workspace_path.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let service = {
                        let mut slot = slot.lock().unwrap();
                        if slot.as_ref().is_none_or(|service| !service.is_connected()) {
                            *slot =
                                Some(CodexService::spawn(super::startup::codex_program(), cwd)?);
                        }
                        slot.as_ref().unwrap().clone()
                    };
                    let result = job(&service);
                    Ok::<_, anyhow::Error>((service, result))
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok((service, result)) => {
                        this.windows.watch_codex(&service, cx);
                        match result {
                            Ok(value) => done(this, value, window, cx),
                            Err(error) => this.report(format!("Codex: {error:#}"), cx),
                        }
                    }
                    Err(error) => {
                        this.fail_openings(AgentKind::Codex, cx);
                        this.report(format!("Codex: {error:#}"), cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
