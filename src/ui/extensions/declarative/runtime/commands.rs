//! Ordered session commands with correlated success/failure replies.

use super::Runtime;
use crate::declarative::protocol::{Outgoing, SessionSetting};
use crate::ui::extensions::host::Host;
use gpui_kit::*;
use std::path::PathBuf;

pub(super) struct Setting {
    generation: u64,
    context: String,
    view: Option<String>,
    id: String,
    session: PathBuf,
    setting: SessionSetting,
}

/// A surface command follows that surface, never another surface that happens
/// to remain in its old window. Context-only commands keep their explicit route.
fn live_context<'a>(
    context: &'a str,
    view: Option<&str>,
    lookup: impl FnOnce(&str) -> Option<&'a str>,
) -> Option<&'a str> {
    match view {
        Some(view) => lookup(view),
        None => Some(context),
    }
}

impl Setting {
    fn route(&self, runtime: &Runtime) -> Option<(String, Host)> {
        let context = live_context(&self.context, self.view.as_deref(), |id| {
            let attached = runtime.views.get(id)?;
            attached.view.upgrade()?;
            Some(attached.context.as_str())
        })?;
        let host = match self.view.as_deref() {
            // During deferred rehost the view already owns its destination
            // Host, even if that window's Scope has not been installed yet.
            Some(view) => runtime.host_for(view)?,
            None => &runtime.scopes.get(context)?.host,
        };
        Some((context.to_owned(), host.clone()))
    }
}

impl Runtime {
    pub(super) fn set_session(
        &mut self,
        context: String,
        view: Option<String>,
        id: String,
        session: PathBuf,
        setting: SessionSetting,
        cx: &mut Context<Self>,
    ) {
        self.settings.push_back(Setting {
            generation: self.generation,
            context,
            view,
            id,
            session,
            setting,
        });
        self.next_setting(cx);
    }

    fn next_setting(&mut self, cx: &mut Context<Self>) {
        if self.setting_running {
            return;
        }
        let (request, api) = loop {
            let Some(request) = self.settings.pop_front() else {
                return;
            };
            if request.generation != self.generation {
                continue;
            }
            let route = request.route(self);
            let api = route.as_ref().and_then(|(_, host)| {
                (host.session_dir(cx).as_ref() == Some(&request.session))
                    .then(|| host.api(cx))
                    .flatten()
            });
            if let Some(api) = api {
                break (request, api);
            }
            self.send(&Outgoing::CommandResult {
                context: route.map_or(request.context, |(context, _)| context),
                id: request.id,
                session_dir: request.session,
                config: None,
                error: Some("session command targets an inactive session".into()),
            });
        };
        self.setting_running = true;
        cx.spawn(async move |this, cx| {
            let session = request.session.clone();
            let setting = request.setting.clone();
            let result = cx
                .background_executor()
                .spawn(async move { api.set_session_setting(&session, setting) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.setting_running = false;
                // A stopped process cannot apply its old completion to a
                // newly mounted view, even when the session path is identical.
                if this.generation != request.generation {
                    this.next_setting(cx);
                    return;
                }
                let route = request.route(this);
                let (config, error) = match result {
                    Ok(config) => (Some(config), None),
                    Err(error) => (None, Some(format!("{error:#}"))),
                };
                if let Some((_, host)) = &route
                    && host.session_dir(cx).as_ref() == Some(&request.session)
                    && let Some(chat) = host.chat(cx)
                {
                    chat.update(cx, |chat, cx| {
                        if let Some(config) = &config {
                            chat.apply_config(config.clone(), cx);
                        } else if let Some(error) = &error {
                            chat.show_notice(error.clone(), cx);
                        }
                    });
                }
                this.send(&Outgoing::CommandResult {
                    context: route.map_or(request.context, |(context, _)| context),
                    id: request.id,
                    session_dir: request.session,
                    config: config.map(Box::new),
                    error,
                });
                this.next_setting(cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::live_context;
    use std::collections::HashMap;

    #[test]
    fn queued_and_inflight_commands_follow_the_view_after_its_source_closes() {
        let mut views = HashMap::from([("moved", "source"), ("other", "source")]);
        let queued = [("source", Some("moved")), ("source", Some("moved"))];
        assert_eq!(
            live_context("source", Some("moved"), |view| views.get(view).copied()),
            Some("source")
        );
        views.insert("moved", "destination");
        // The source still owns a different surface. It must never receive
        // either command's dispatch or completion after the transfer.
        for (context, view) in queued {
            assert_eq!(
                live_context(context, view, |view| views.get(view).copied()),
                Some("destination")
            );
        }
        views.remove("moved");
        assert_eq!(
            live_context("source", Some("moved"), |view| views.get(view).copied()),
            None
        );
        assert_eq!(
            live_context("source", None, |view| views.get(view).copied()),
            Some("source")
        );
    }
}
