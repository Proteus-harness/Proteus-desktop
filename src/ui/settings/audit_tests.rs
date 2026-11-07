use super::*;
use crate::conversations::AgentKind;
use crate::tabs::{ChatKey, PaneId};
use crate::ui::extensions::OpenDocument;
use crate::ui::workspace::Workspace;

mod catalog_save;

fn snapshot(provider: &str) -> ConfigBuilderSnapshot {
    ConfigBuilderSnapshot {
        writable: true,
        active_provider: Some(provider.into()),
        permission_mode: "normal".into(),
        ..Default::default()
    }
}

fn context() -> ::gpui::TestAppContext {
    let cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    cx
}

#[::core::prelude::v1::test]
fn save_acknowledges_submitted_values_while_newer_edits_stay_dirty() {
    let mut cx = context();
    let window = cx.add_empty_window();
    let view = window.update(|window, cx| {
        cx.new(|cx| {
            SettingsView::new(
                WeakEntity::new_invalid(),
                SettingsContext::for_chat(None, || None),
                window,
                cx,
            )
        })
    });
    view.update(&mut cx, |view, cx| {
        view.accept(snapshot("initial"));
        view.update_draft(cx, |draft| draft.provider = Some("submitted".into()));
        let submitted = view.draft.clone();
        let submitted_revision = view.edit_revision;
        // The successful server response below acknowledges only this request.
        let sent = submitted.changes(&view.baseline);
        view.saving = true;
        view.update_draft(cx, |draft| draft.set_tool("newer-tool", true));
        view.saving = false;
        view.accept_save(snapshot("submitted"), submitted_revision);
        assert_eq!(sent.active_provider.as_deref(), Some("submitted"));
        assert!(sent.tools_enabled.is_none());
        assert_eq!(view.baseline.provider.as_deref(), Some("submitted"));
        assert!(view.draft.tools.contains("newer-tool"));
        assert!(view.dirty());
        let later = view.draft.changes(&view.baseline);
        assert!(later.active_provider.is_none());
        assert_eq!(later.tools_enabled, Some(vec!["newer-tool".into()]));
    });
}

#[::core::prelude::v1::test]
fn save_does_not_destroy_invalid_text_entered_after_submission() {
    let mut cx = context();
    let window = cx.add_empty_window();
    let view = window.update(|window, cx| {
        cx.new(|cx| {
            SettingsView::new(
                WeakEntity::new_invalid(),
                SettingsContext::for_chat(None, || None),
                window,
                cx,
            )
        })
    });
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(snapshot("initial"));
            view.draft.provider = Some("submitted".into());
            let submitted_revision = view.edit_revision;
            view.ensure_json("tool", "synthetic", window, cx);
            let key = ("tool".into(), "synthetic".into());
            let input = view.editors.json[&key].input.clone();
            input.update(cx, |input, cx| input.set_value("{invalid", window, cx));
            view.json_changed(&key, "{invalid", cx);
            view.accept_save(snapshot("submitted"), submitted_revision);
            assert_eq!(view.editors.json[&key].input.entity_id(), input.entity_id());
            assert_eq!(input.read(cx).value().as_ref(), "{invalid");
            assert!(view.editors.has_errors());
            assert!(view.has_unsaved_changes());
            assert!(!view.check_close(cx));
        })
    });
}

#[::core::prelude::v1::test]
fn save_preserves_newer_editor_buffer_even_when_its_parsed_value_is_unchanged() {
    let mut cx = context();
    let window = cx.add_empty_window();
    let view = window.update(|window, cx| {
        cx.new(|cx| {
            SettingsView::new(
                WeakEntity::new_invalid(),
                SettingsContext::for_chat(None, || None),
                window,
                cx,
            )
        })
    });
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(snapshot("submitted"));
            view.ensure_json("tool", "synthetic", window, cx);
            let key = ("tool".into(), "synthetic".into());
            let input = view.editors.json[&key].input.clone();
            let submitted_revision = view.edit_revision;
            input.update(cx, |input, cx| input.set_value("{  }", window, cx));
            view.json_changed(&key, "{  }", cx);
            view.accept_save(snapshot("submitted"), submitted_revision);
            assert_eq!(view.editors.json[&key].input.entity_id(), input.entity_id());
            assert_eq!(input.read(cx).value().as_ref(), "{  }");
            assert!(!view.dirty());
        })
    });
}

#[::core::prelude::v1::test]
fn every_explicit_destination_preserves_dirty_settings_and_pending_save() {
    let mut cx = context();
    let window = cx.add_empty_window();
    let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
    window.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_settings(window, cx);
            let settings = workspace.settings.clone().unwrap();
            settings.update(cx, |settings, _| {
                settings.accept(snapshot("initial"));
                settings.draft.provider = Some("unsaved".into());
            });
            let before = workspace.layout.clone();
            workspace.toggle_settings(window, cx);
            workspace.open_chat(
                ChatKey::Codex("never-resume".into()),
                OpenDocument {
                    preview: false,
                    focus: true,
                },
                window,
                cx,
            );
            workspace.open_terminal(None, window, cx);
            workspace.open_document(
                "never-start:document",
                OpenDocument {
                    preview: false,
                    focus: true,
                },
                window,
                cx,
            );
            workspace.activate_tab(PaneId(0), "never-start:document", window, cx);
            workspace.new_session(None, window, cx);
            workspace.open_project(
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
                window,
                cx,
            );
            assert_eq!(workspace.settings.as_ref(), Some(&settings));
            assert_eq!(workspace.layout, before);
            assert!(workspace.opening.is_empty());
            assert!(workspace.creating_chat.is_none());
            settings.update(cx, |settings, cx| {
                settings.discard(cx);
                settings.saving = true;
            });
            workspace.toggle_settings(window, cx);
            assert_eq!(workspace.settings.as_ref(), Some(&settings));
            assert!(
                settings
                    .read(cx)
                    .feedback
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("сохраняется")
            );
        })
    });
}

#[::core::prelude::v1::test]
fn delayed_project_response_cannot_replace_settings_or_a_new_chat_intention() {
    let mut cx = context();
    let window = cx.add_empty_window();
    let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
    window.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            assert!(workspace.begin_navigation(cx));
            let old_project = workspace.navigation_generation;
            workspace.open_settings(window, cx);
            let settings = workspace.settings.clone().unwrap();
            settings.update(cx, |settings, _| {
                settings.accept(snapshot("initial"));
                settings.draft.provider = Some("unsaved".into());
            });
            workspace.finish_project_navigation(
                "/old-project".into(),
                AgentKind::Proteus,
                old_project,
                window,
                cx,
            );
            assert_eq!(workspace.settings.as_ref(), Some(&settings));
            assert!(settings.read(cx).dirty());
            settings.update(cx, |settings, cx| settings.discard(cx));
            assert!(workspace.begin_navigation(cx));
            let old_project = workspace.navigation_generation;
            assert!(workspace.begin_navigation(cx));
            let new_chat = workspace.begin_chat_creation();
            workspace.finish_project_navigation(
                "/old-project".into(),
                AgentKind::Codex,
                old_project,
                window,
                cx,
            );
            assert!(
                workspace
                    .creating_chat
                    .as_ref()
                    .is_some_and(|ticket| std::rc::Rc::ptr_eq(ticket, &new_chat))
            );
            assert!(workspace.settings.is_none());
            assert!(workspace.sessions.is_empty());
        })
    });
}

#[::core::prelude::v1::test]
fn tools_and_disabled_hooks_prepare_editors_without_visiting_modules() {
    use proteus_contracts::app_protocol::config_builder::{
        ConfigBuilderModule, ConfigBuilderPlugin, ConfigBuilderPluginExport,
    };
    let mut cx = context();
    let window = cx.add_empty_window();
    window.update(|window, cx| {
        let view = cx.new(|cx| {
            SettingsView::new(
                WeakEntity::new_invalid(),
                SettingsContext::for_chat(None, || None),
                window,
                cx,
            )
        });
        view.update(cx, |view, cx| {
            let mut snapshot = snapshot("initial");
            snapshot.plugins.push(ConfigBuilderPlugin {
                id: "fixture".into(),
                command: "synthetic".into(),
                description: None,
                tool_packs: vec![],
                exports: vec![ConfigBuilderPluginExport {
                    slot: "tool".into(),
                    id: "pack".into(),
                    active: true,
                    description: None,
                    config_schema: None,
                }],
            });
            snapshot.hook_modules.push(ConfigBuilderModule {
                id: "disabled".into(),
                slot: "hook".into(),
                ..Default::default()
            });
            snapshot
                .module_config
                .entry("tool".into())
                .or_default()
                .insert("pack".into(), serde_json::json!({"path":"fixture"}));
            snapshot
                .module_config
                .entry("hook".into())
                .or_default()
                .insert("disabled".into(), serde_json::json!({"text":"fixture"}));
            view.accept(snapshot);
            assert!(view.draft.hooks.is_empty());
            view.page = PageId::Tools;
            view.prepare_editors(window, cx);
            assert!(
                view.editors
                    .json
                    .contains_key(&("tool".into(), "pack".into()))
            );
            view.page = PageId::Hooks;
            view.prepare_editors(window, cx);
            assert!(
                view.editors
                    .json
                    .contains_key(&("hook".into(), "disabled".into()))
            );
        });
    });
}
