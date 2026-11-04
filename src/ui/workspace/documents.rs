// Адаптировано из Zed: crates/editor/src/editor.rs (FileHandleChanged).
//! Generic document identity changes within one trusted server and session.
use super::*;
use crate::tabs;
use crate::ui::navigation::Destination;
use std::path::Path;

impl Workspace {
    pub(in crate::ui) fn has_session_document(
        &self,
        connection: u64,
        session: &Path,
        id: &str,
    ) -> bool {
        if self
            .api()
            .is_none_or(|api| api.connection_id() != connection)
        {
            return false;
        }
        let root = tabs::chat(session);
        if self.spaces.active_root() == Some(root.as_str()) {
            return self.layout.contains(id);
        }
        self.spaces
            .root_layouts()
            .any(|(owner, layout)| owner == root && layout.contains(id))
    }

    pub(in crate::ui) fn retarget_document(
        &mut self,
        connection: u64,
        session: &Path,
        extension: &str,
        from: &str,
        to: &str,
        destination_elsewhere: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if from == to
            || self
                .api()
                .is_none_or(|api| api.connection_id() != connection)
        {
            return;
        }
        let root = tabs::chat(session);
        let old = tabs::document(extension, from);
        let new = tabs::document(extension, to);
        self.spaces.save_active(self.layout.clone());
        let active = self.spaces.active_root() == Some(root.as_str());
        let collision_focused = active
            && (destination_elsewhere || self.layout.contains(&new))
            && self.layout.active(self.layout.focused) == Some(old.as_str());
        let updated = if destination_elsewhere {
            self.spaces.retarget_document_to_existing(&root, &old, &new)
        } else {
            self.spaces.retarget_document(&root, &old, &new)
        };
        if !updated {
            return;
        }
        if active {
            self.layout = self.spaces.active_layout().clone();
            if destination_elsewhere {
                self.documents.remove(&old);
            }
            retarget_map(&mut self.documents, &old, &new);
            if destination_elsewhere {
                self.document_titles.remove(&old);
            }
            retarget_map(&mut self.document_titles, &old, &new);
            retarget_history(&mut self.navigation, &old, &new);
            retarget_strips(&mut self.tab_strips, &old, &new);
        } else if let Some(parked) = self.parked_spaces.get_mut(&Some(root)) {
            if destination_elsewhere {
                parked.documents.remove(&old);
                parked.titles.remove(&old);
            }
            retarget_map(&mut parked.documents, &old, &new);
            retarget_map(&mut parked.titles, &old, &new);
            retarget_history(&mut parked.navigation, &old, &new);
            retarget_strips(&mut parked.strips, &old, &new);
        }
        self.windows.persist(cx.entity_id(), &self.spaces, cx);
        if collision_focused && self.settings.is_none() {
            self.focus_active(window, cx);
        }
        cx.emit(super::super::extensions::HostEvent::NavigationChanged);
        cx.notify();
    }
}

fn retarget_map<T>(map: &mut HashMap<String, T>, from: &str, to: &str) {
    if let Some(value) = map.remove(from) {
        map.entry(to.to_owned()).or_insert(value);
    }
}

fn retarget_history(history: &mut crate::navigation::History<Destination>, from: &str, to: &str) {
    history.map(|destination| {
        if let Destination::Tab(id) = destination
            && id == from
        {
            *id = to.to_owned();
        }
    });
}

fn retarget_strips(
    strips: &mut HashMap<PaneId, (ScrollHandle, Option<String>)>,
    from: &str,
    to: &str,
) {
    for (_, selected) in strips.values_mut() {
        if selected.as_deref() == Some(from) {
            *selected = Some(to.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Document {
        focus: FocusHandle,
    }
    impl Render for Document {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    fn surface(cx: &mut App) -> SurfaceView {
        let view = cx.new(|cx| Document {
            focus: cx.focus_handle(),
        });
        let focus = view.read(cx).focus.clone();
        SurfaceView {
            view: view.into(),
            focus,
            rehost: Rc::new(|_, _, _| {}),
        }
    }
    fn init() -> ::gpui::TestAppContext {
        let cx = ::gpui::TestAppContext::single();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        cx
    }
    fn seed(workspace: &mut Workspace, session: &str, cx: &mut Context<Workspace>) -> EntityId {
        workspace.spaces.select(&tabs::chat(Path::new(session)));
        workspace.layout = workspace.spaces.active_layout().clone();
        workspace.layout.open("fixture:old", |_| false);
        workspace.layout.open("other:old", |_| false);
        workspace.layout.activate(PaneId(0), "fixture:old");
        workspace.layout.preview = Some("fixture:old".into());
        workspace.spaces.save_active(workspace.layout.clone());
        let document = surface(cx);
        let identity = document.view.entity_id();
        workspace.documents.insert("fixture:old".into(), document);
        workspace
            .document_titles
            .insert("fixture:old".into(), "Caption".into());
        workspace
            .navigation
            .visit(Destination::Tab("fixture:old".into()));
        workspace
            .navigation
            .visit(Destination::Tab("other:old".into()));
        identity
    }

    #[::core::prelude::v1::test]
    fn retarget_keeps_entity_caption_preview_navigation_and_isolates_connection_session_extension()
    {
        let mut cx = init();
        let window = cx.add_empty_window();
        let api = Api::new("http://127.0.0.1:1", "synthetic");
        let unrelated = Api::new("http://127.0.0.1:1", "synthetic");
        let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
        window.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.api = Some(api.clone());
                let identity = seed(workspace, "/one", cx);
                let before = workspace.layout.clone();
                workspace.retarget_document(
                    unrelated.connection_id(),
                    Path::new("/one"),
                    "fixture",
                    "old",
                    "new",
                    false,
                    window,
                    cx,
                );
                assert_eq!(workspace.layout, before);
                workspace.retarget_document(
                    api.connection_id(),
                    Path::new("/two"),
                    "fixture",
                    "old",
                    "new",
                    false,
                    window,
                    cx,
                );
                assert_eq!(workspace.layout, before);
                workspace.retarget_document(
                    api.connection_id(),
                    Path::new("/one"),
                    "fixture",
                    "old",
                    "new",
                    false,
                    window,
                    cx,
                );
                assert_eq!(
                    workspace.documents["fixture:new"].view.entity_id(),
                    identity
                );
                assert!(!workspace.documents.contains_key("fixture:old"));
                assert_eq!(workspace.document_titles["fixture:new"].as_ref(), "Caption");
                assert_eq!(workspace.layout.focused, before.focused);
                assert_eq!(workspace.layout.preview.as_deref(), Some("fixture:new"));
                assert_eq!(
                    workspace.layout.active(workspace.layout.focused),
                    Some("fixture:new")
                );
                assert!(workspace.layout.contains("other:old"));
                assert_eq!(
                    workspace.navigation.back(|_| true),
                    Some(Destination::Tab("fixture:new".into()))
                );
                assert!(workspace.spaces.validate().is_ok());
            })
        });
    }

    #[::core::prelude::v1::test]
    fn retarget_parked_session_preserves_foreground_and_existing_destination_entity() {
        let mut cx = init();
        let window = cx.add_empty_window();
        let api = Api::new("http://127.0.0.1:1", "synthetic");
        let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
        window.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.api = Some(api.clone());
                seed(workspace, "/one", cx);
                workspace.layout.open("fixture:new", |_| false);
                workspace.layout.keep("fixture:new");
                let destination = surface(cx);
                let identity = destination.view.entity_id();
                workspace
                    .documents
                    .insert("fixture:new".into(), destination);
                workspace.spaces.save_active(workspace.layout.clone());
                // Park the first area while an unrelated session is displayed.
                let mut parked = super::super::spaces::SpaceViews::default();
                parked.documents = std::mem::take(&mut workspace.documents);
                parked.titles = std::mem::take(&mut workspace.document_titles);
                workspace
                    .parked_spaces
                    .insert(Some("chat:/one".into()), parked);
                seed(workspace, "/two", cx);
                let foreground = workspace.layout.clone();
                let foreground_entity = workspace.documents["fixture:old"].view.entity_id();
                workspace.retarget_document(
                    api.connection_id(),
                    Path::new("/one"),
                    "fixture",
                    "old",
                    "new",
                    false,
                    window,
                    cx,
                );
                assert_eq!(workspace.layout, foreground);
                assert_eq!(
                    workspace.documents["fixture:old"].view.entity_id(),
                    foreground_entity
                );
                let parked = &workspace.parked_spaces[&Some("chat:/one".into())];
                assert_eq!(parked.documents["fixture:new"].view.entity_id(), identity);
                assert!(!parked.documents.contains_key("fixture:old"));
                let old = workspace
                    .spaces
                    .root_layouts()
                    .find(|(root, _)| *root == "chat:/one")
                    .unwrap()
                    .1;
                assert!(!old.contains("fixture:old"));
                assert!(old.contains("fixture:new"));
                assert!(workspace.spaces.validate().is_ok());
            })
        });
    }
    #[::core::prelude::v1::test]
    fn destination_in_another_window_keeps_ownership_without_duplicating_source() {
        let mut cx = init();
        let source_window = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        let destination_window = cx
            .add_empty_window()
            .update(|window, _| window.window_handle());
        let api = Api::new("http://127.0.0.1:1", "synthetic");
        let source = source_window
            .update(&mut cx, |_, window, cx| {
                cx.new(|cx| Workspace::test_empty(window, cx))
            })
            .unwrap();
        let destination = destination_window
            .update(&mut cx, |_, window, cx| {
                cx.new(|cx| Workspace::test_empty(window, cx))
            })
            .unwrap();
        let identity = destination_window
            .update(&mut cx, |_, window, cx| {
                destination.update(cx, |workspace, cx| {
                    workspace.api = Some(api.clone());
                    let identity = seed(workspace, "/one", cx);
                    workspace.retarget_document(
                        api.connection_id(),
                        Path::new("/one"),
                        "fixture",
                        "old",
                        "new",
                        false,
                        window,
                        cx,
                    );
                    assert!(workspace.has_session_document(
                        api.connection_id(),
                        Path::new("/one"),
                        "fixture:new"
                    ));
                    identity
                })
            })
            .unwrap();
        source_window
            .update(&mut cx, |_, window, cx| {
                source.update(cx, |workspace, cx| {
                    workspace.api = Some(api.clone());
                    seed(workspace, "/one", cx);
                    workspace.retarget_document(
                        api.connection_id(),
                        Path::new("/one"),
                        "fixture",
                        "old",
                        "new",
                        true,
                        window,
                        cx,
                    );
                    assert!(!workspace.layout.contains("fixture:old"));
                    assert!(!workspace.layout.contains("fixture:new"));
                    assert!(!workspace.documents.contains_key("fixture:old"));
                    assert!(!workspace.documents.contains_key("fixture:new"));
                    assert_eq!(
                        workspace.layout.active(workspace.layout.focused),
                        Some("other:old")
                    );
                    assert_eq!(
                        workspace.navigation.back(|_| true),
                        Some(Destination::Tab("fixture:new".into()))
                    );
                    assert!(workspace.spaces.validate().is_ok());
                })
            })
            .unwrap();
        destination.read_with(&cx, |workspace, _| {
            assert_eq!(
                workspace.documents["fixture:new"].view.entity_id(),
                identity
            );
            assert_eq!(
                workspace.layout.active(workspace.layout.focused),
                Some("fixture:new")
            );
        });
    }
}
