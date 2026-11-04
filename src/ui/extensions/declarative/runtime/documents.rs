//! Retarget opaque arguments only within the addressed connection and session.
use super::*;

impl Runtime {
    pub(super) fn retarget_document(
        &self,
        context: &str,
        from: String,
        to: String,
        cx: &mut Context<Self>,
    ) {
        if from == to
            || !self.lifetime.session()
            || self.extension.manifest.surfaces.document.is_none()
        {
            return;
        }
        let Some(host) = self.scopes.get(context).map(|scope| &scope.host) else {
            return;
        };
        let (Some(session), Some(api)) = (host.session_dir(cx), host.api(cx)) else {
            return;
        };
        let connection = api.connection_id();
        let extension = self.extension_id();
        // The sender is among the targets; release its entity borrow first.
        cx.defer(move |cx| {
            for runtime in invalidation::runtimes(extension, cx) {
                runtime.update(cx, |runtime, cx| {
                    runtime.retarget_arguments(connection, &session, &from, &to, cx);
                });
            }
            let workspaces: Vec<_> = cx
                .windows()
                .into_iter()
                .filter_map(|window| {
                    let window = window.downcast::<crate::ui::launcher::Launcher>()?;
                    window.read(cx).ok()?.ready_workspace()
                })
                .collect();
            let destination = crate::tabs::document(extension, &to);
            let existing = workspaces
                .iter()
                .filter(|workspace| {
                    workspace
                        .read(cx)
                        .has_session_document(connection, &session, &destination)
                })
                .map(|workspace| workspace.entity_id())
                .min_by_key(|id| id.to_string());
            for workspace in workspaces {
                let destination_elsewhere =
                    existing.is_some_and(|owner| owner != workspace.entity_id());
                let window = workspace.read(cx).window_handle;
                let _ = window.update(cx, |_, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        workspace.retarget_document(
                            connection,
                            &session,
                            extension,
                            &from,
                            &to,
                            destination_elsewhere,
                            window,
                            cx,
                        );
                    })
                });
            }
        });
    }

    fn retarget_arguments(
        &mut self,
        connection: u64,
        session: &std::path::Path,
        from: &str,
        to: &str,
        cx: &mut Context<Self>,
    ) {
        let views: Vec<_> = self
            .views
            .iter()
            .filter_map(|(id, attached)| {
                let host = self.host_for(id)?;
                (attached.surface == SurfaceKind::Document
                    && attached.argument.as_deref() == Some(from)
                    && host.session_dir(cx).as_deref() == Some(session)
                    && host
                        .api(cx)
                        .is_some_and(|api| api.connection_id() == connection))
                .then(|| id.clone())
            })
            .collect();
        for id in views {
            self.views.get_mut(&id).unwrap().argument = Some(to.to_owned());
            self.render(&id, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::Api;
    use crate::tabs::ChatKey;
    use crate::ui::workspace::Workspace;

    #[::core::prelude::v1::test]
    fn attached_document_arguments_follow_exact_session_connection_without_changing_view_ids() {
        let mut cx = ::gpui::TestAppContext::single();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        let handle = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        let sibling_handle = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        let api = Api::new("http://127.0.0.1:1", "synthetic");
        let foreign = Api::new("http://127.0.0.1:1", "synthetic");
        let mut create = |api: Api, handle: AnyWindowHandle| {
            handle
                .update(&mut cx, |_, window, cx| {
                    cx.new(|cx| {
                        let mut workspace = Workspace::test_empty(window, cx);
                        workspace.test_set_api(api);
                        workspace
                    })
                })
                .unwrap()
        };
        let workspace = create(api.clone(), handle);
        let unrelated = create(foreign, handle);
        let sibling = create(api.clone(), sibling_handle);
        let installed = Box::leak(Box::new(Installed {
            dir: PathBuf::from("/synthetic-extension"),
            revision: 0,
            manifest: serde_json::from_value(serde_json::json!({
                "protocol":4, "id":"retarget-fixture", "title":"Fixture", "command":["never-start"],
                "surfaces":{"document":{}}
            }))
            .unwrap(),
        }));
        let host = Host::new(workspace.downgrade(), "retarget-fixture")
            .for_space(ChatKey::Proteus("/one".into()));
        let runtime = handle
            .update(&mut cx, |_, window, cx| {
                cx.new(|cx| Runtime::new(installed, host.clone(), window, cx))
            })
            .unwrap();
        runtime.update(&mut cx, |runtime, cx| {
            for (id, host, surface) in [
                ("same", host.clone(), SurfaceKind::Document),
                (
                    "sibling",
                    Host::new(sibling.downgrade(), "retarget-fixture")
                        .for_space(ChatKey::Proteus("/one".into())),
                    SurfaceKind::Document,
                ),
                (
                    "other-session",
                    host.clone().for_space(ChatKey::Proteus("/two".into())),
                    SurfaceKind::Document,
                ),
                (
                    "other-connection",
                    Host::new(unrelated.downgrade(), "retarget-fixture")
                        .for_space(ChatKey::Proteus("/one".into())),
                    SurfaceKind::Document,
                ),
                ("panel", host.clone(), SurfaceKind::Panel),
            ] {
                runtime.views.insert(
                    id.into(),
                    Attached {
                        view: WeakEntity::new_invalid(),
                        surface,
                        argument: Some("old".into()),
                        content: None,
                        context: host.scope(),
                        host,
                        window: if id == "sibling" {
                            sibling_handle
                        } else {
                            handle
                        },
                        route_generation: 7,
                        queue: ContentQueue::default(),
                    },
                );
            }
            runtime.retarget_arguments(
                api.connection_id(),
                std::path::Path::new("/one"),
                "old",
                "new",
                cx,
            );
            for id in ["same", "sibling"] {
                assert_eq!(runtime.views[id].argument.as_deref(), Some("new"));
                assert_eq!(runtime.views[id].route_generation, 7);
            }
            for id in ["other-session", "other-connection", "panel"] {
                assert_eq!(runtime.views[id].argument.as_deref(), Some("old"));
            }
            assert_eq!(runtime.views.len(), 5);
        });
    }
}
