use super::*;
use crate::settings::mcp::McpText;
use crate::settings::reconcile::CatalogChange;
use gpui_kit::component::input::InputEvent;
use serde_json::json;

fn with_addons(addons: serde_json::Value) -> ConfigBuilderSnapshot {
    ConfigBuilderSnapshot {
        addon_settings: serde_json::from_value(addons).unwrap(),
        ..snapshot("initial")
    }
}

fn view(cx: &mut ::gpui::TestAppContext) -> (&mut ::gpui::VisualTestContext, Entity<SettingsView>) {
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
    (window, view)
}

#[::core::prelude::v1::test]
fn delayed_catalog_install_and_remove_preserve_late_edits_through_next_save() {
    for installing in [true, false] {
        let mut cx = context();
        let (_, view) = view(&mut cx);
        view.update(&mut cx, |view, cx| {
            let empty = with_addons(json!({"addons":{},"mcp_servers":[]}));
            let installed =
                with_addons(json!({"addons":{"plugins":[{"path":"catalog"}]},"mcp_servers":[]}));
            let (before, accepted) = if installing {
                (empty, installed)
            } else {
                (installed, empty)
            };
            view.accept(before);
            let submitted_revision = view.edit_revision;
            let change = CatalogChange {
                before: view.baseline.addons.clone(),
                replacement: None,
            };
            view.catalogs.busy = true;
            // This edit happens after dispatch and before the delayed response.
            view.update_draft(cx, |draft| {
                draft.set_tool("late-tool", true);
                draft.add_package("local".into());
            });
            view.finish_catalog_profile_write(
                Ok(accepted),
                submitted_revision,
                &change,
                |_| {},
                "done",
                cx,
            );
            assert!(!view.catalogs.busy);
            assert!(view.dirty());
            assert!(view.draft.tools.contains("late-tool"));
            let next = view.draft.changes(&view.baseline);
            let plugins = &next.addon_settings.unwrap().addons.plugins;
            assert!(
                plugins
                    .iter()
                    .any(|plugin| plugin.path == std::path::Path::new("local"))
            );
            assert_eq!(
                plugins
                    .iter()
                    .any(|plugin| plugin.path == std::path::Path::new("catalog")),
                installing
            );
            let saved = ConfigBuilderSnapshot {
                addon_settings: view.draft.addons.clone(),
                tools_enabled: view.draft.tools.iter().cloned().collect(),
                ..snapshot("initial")
            };
            view.accept_save(saved, view.edit_revision);
            assert!(!view.dirty());
            assert_eq!(
                view.baseline
                    .addons
                    .addons
                    .plugins
                    .iter()
                    .any(|plugin| plugin.path == std::path::Path::new("catalog")),
                installing
            );
        });
    }
}

#[::core::prelude::v1::test]
fn catalog_failure_keeps_the_draft_and_receipt_retry_acknowledges_only_success() {
    let mut cx = context();
    let (_, view) = view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(snapshot("initial"));
        let change = CatalogChange {
            before: view.baseline.addons.clone(),
            replacement: None,
        };
        let revision = view.edit_revision;
        view.catalogs.busy = true;
        view.update_draft(cx, |draft| draft.set_tool("late-tool", true));
        view.finish_catalog_profile_write(
            Err(anyhow::anyhow!("synthetic validation error")),
            revision,
            &change,
            |_| panic!("failed save must not keep a receipt"),
            "done",
            cx,
        );
        assert!(!view.catalogs.busy);
        assert!(view.baseline.tools.is_empty());
        assert!(view.draft.tools.contains("late-tool"));
        assert!(
            view.catalogs
                .message
                .as_ref()
                .unwrap()
                .1
                .contains("synthetic validation error")
        );
        view.discard(cx);
        let receipts = std::rc::Rc::new(std::cell::Cell::new(0));
        let kept = receipts.clone();
        view.catalogs.busy = true;
        view.finish_catalog_profile_write(
            Ok(with_addons(
                json!({"addons":{"plugins":[{"path":"catalog"}]},"mcp_servers":[]}),
            )),
            view.edit_revision,
            &change,
            move |_| kept.set(kept.get() + 1),
            "done",
            cx,
        );
        assert_eq!(receipts.get(), 1);
        assert!(!view.dirty());
        assert_eq!(view.draft.addons.addons.plugins.len(), 1);
    });
}

#[::core::prelude::v1::test]
fn pending_catalog_mutation_keeps_clean_settings_open() {
    let mut cx = context();
    let (_, view) = view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(snapshot("initial"));
        assert!(!view.dirty());
        view.catalogs.busy = true;
        assert!(!view.check_close(cx));
        assert!(
            view.feedback
                .as_ref()
                .unwrap()
                .text
                .contains("Операция с пакетом")
        );
        view.catalogs.busy = false;
        assert!(view.check_close(cx));
    });
}

#[::core::prelude::v1::test]
fn catalog_ack_keeps_invalid_module_editor_and_the_installed_binding() {
    let mut cx = context();
    let (window, view) = view(&mut cx);
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(snapshot("initial"));
            let revision = view.edit_revision;
            let change = CatalogChange {
                before: view.baseline.addons.clone(),
                replacement: None,
            };
            view.ensure_json("tool", "synthetic", window, cx);
            let key = ("tool".into(), "synthetic".into());
            let input = view.editors.json[&key].input.clone();
            input.update(cx, |input, cx| input.set_value("{invalid", window, cx));
            view.json_changed(&key, "{invalid", cx);
            view.finish_catalog_profile_write(
                Ok(with_addons(
                    json!({"addons":{"plugins":[{"path":"catalog"}]},"mcp_servers":[]}),
                )),
                revision,
                &change,
                |_| {},
                "done",
                cx,
            );
            assert_eq!(view.editors.json[&key].input.entity_id(), input.entity_id());
            assert_eq!(input.read(cx).value().as_ref(), "{invalid");
            assert!(view.editors.has_errors());
            assert_eq!(view.draft.addons, view.baseline.addons);
            assert_eq!(view.draft.addons.addons.plugins.len(), 1);
        });
    });
}

#[::core::prelude::v1::test]
fn catalog_removal_rebinds_retained_mcp_editor_and_reports_raw_overlap() {
    let mut cx = context();
    let (window, view) = view(&mut cx);
    let input = window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(with_addons(json!({"addons":{},"mcp_servers":[
                {"name":"remove","command":"one"}, {"name":"keep","command":"two"}]})));
            let revision = view.edit_revision;
            let change = CatalogChange {
                before: view.baseline.addons.clone(),
                replacement: None,
            };
            view.open.insert(super::super::mcp::mcp_key("keep"));
            view.prepare_mcp_forms(window, cx);
            let super::super::editors::TextInput::Line(input) =
                view.editors.mcp[&1].inputs[&McpText::TimeoutMs].clone()
            else {
                panic!("single line timeout");
            };
            input.update(cx, |state, cx| state.set_value("invalid-ms", window, cx));
            view.mcp_text_changed(1, McpText::TimeoutMs, "invalid-ms", cx);
            view.finish_catalog_profile_write(
                Ok(with_addons(
                    json!({"addons":{},"mcp_servers":[{"name":"keep","command":"two"}]}),
                )),
                revision,
                &change,
                |_| {},
                "done",
                cx,
            );
            let super::super::editors::TextInput::Line(retained) =
                &view.editors.mcp[&0].inputs[&McpText::TimeoutMs]
            else {
                panic!("single line timeout");
            };
            assert_eq!(retained.entity_id(), input.entity_id());
            assert_eq!(input.read(cx).value().as_ref(), "invalid-ms");
            assert!(!view.editors.mcp.contains_key(&1));
            input
        })
    });
    // Exercise the retained input's real subscription after its index moved.
    window.update(|window, cx| {
        input.update(cx, |state, cx| {
            state.set_value("17", window, cx);
            cx.emit(InputEvent::Change);
        });
    });
    cx.run_until_parked();
    view.update(&mut cx, |view, cx| {
        assert_eq!(view.draft.addons.mcp_servers[0].name, "keep");
        assert_eq!(view.draft.addons.mcp_servers[0].timeout_ms, Some(17));
        assert!(!view.editors.has_errors());
        let change = CatalogChange {
            before: view.baseline.addons.clone(),
            replacement: None,
        };
        let revision = view.edit_revision.wrapping_sub(1);
        view.finish_catalog_profile_write(
            Ok(with_addons(json!({"addons":{},"mcp_servers":[]}))),
            revision,
            &change,
            |_| {},
            "done",
            cx,
        );
        assert!(view.baseline.addons.mcp_servers.is_empty());
        assert_eq!(view.draft.addons.mcp_servers.len(), 1);
        assert!(
            view.catalog_conflict
                .as_ref()
                .is_some_and(|text| text.contains("Конфликт: MCP keep"))
        );
        view.update_draft(cx, |draft| draft.set_tool("unrelated", true));
        assert!(
            view.catalog_conflict.is_some(),
            "unrelated edits must not dismiss the conflict"
        );
        assert!(view.dirty());
        view.discard(cx);
        assert!(view.catalog_conflict.is_none());
    });
}

#[::core::prelude::v1::test]
fn delayed_catalog_ack_keeps_duplicate_mcp_names_and_distinct_raw_editors() {
    for remove_other in [false, true] {
        let mut cx = context();
        let (window, view) = view(&mut cx);
        let (first, second) = window.update(|window, cx| {
            view.update(cx, |view, cx| {
                let initial = with_addons(json!({"addons":{},"mcp_servers":[
                    {"name":"remove","command":"other"},
                    {"name":"a","command":"same"}, {"name":"b","command":"same"}]}));
                view.accept(initial.clone());
                let revision = view.edit_revision;
                let change = CatalogChange {
                    before: view.baseline.addons.clone(),
                    replacement: None,
                };
                view.catalogs.busy = true;
                for name in ["a", "b"] {
                    view.open.insert(super::super::mcp::mcp_key(name));
                }
                view.prepare_mcp_forms(window, cx);
                let super::super::editors::TextInput::Line(name) =
                    view.editors.mcp[&2].inputs[&McpText::Name].clone()
                else {
                    panic!("line name");
                };
                name.update(cx, |state, cx| state.set_value("a", window, cx));
                view.mcp_text_changed(2, McpText::Name, "a", cx);
                let mut inputs = Vec::new();
                for (ix, raw) in [(1, "first-invalid"), (2, "second-invalid")] {
                    let super::super::editors::TextInput::Line(input) =
                        view.editors.mcp[&ix].inputs[&McpText::TimeoutMs].clone()
                    else {
                        panic!("line timeout");
                    };
                    input.update(cx, |state, cx| state.set_value(raw, window, cx));
                    view.mcp_text_changed(ix, McpText::TimeoutMs, raw, cx);
                    inputs.push(input);
                }
                // Both parsed rows are now identical, but their input buffers
                // and subscriptions still have independent logical owners.
                assert_eq!(
                    view.draft.addons.mcp_servers[1],
                    view.draft.addons.mcp_servers[2]
                );
                let mut accepted = initial;
                if remove_other {
                    accepted.addon_settings.mcp_servers.remove(0);
                } else {
                    accepted.addon_settings.addons.plugins =
                        serde_json::from_value(json!([{"path":"catalog"}])).unwrap();
                }
                view.finish_catalog_profile_write(
                    Ok(accepted),
                    revision,
                    &change,
                    |_| {},
                    "done",
                    cx,
                );
                let start = if remove_other { 0 } else { 1 };
                assert_eq!(view.editors.mcp.len(), 2);
                for (offset, input) in inputs.iter().enumerate() {
                    let form = &view.editors.mcp[&(start + offset)];
                    let super::super::editors::TextInput::Line(retained) =
                        &form.inputs[&McpText::TimeoutMs]
                    else {
                        panic!("line timeout");
                    };
                    assert_eq!(retained.entity_id(), input.entity_id());
                    assert!(form.errors.contains_key(&McpText::TimeoutMs));
                }
                assert_eq!(inputs[0].read(cx).value().as_ref(), "first-invalid");
                assert_eq!(inputs[1].read(cx).value().as_ref(), "second-invalid");
                (inputs[0].clone(), inputs[1].clone())
            })
        });
        // Exercise both real callbacks after the ACK. Neither input may edit
        // its identical-name neighbour, including after an earlier row left.
        window.update(|window, cx| {
            for (input, text) in [(&first, "11"), (&second, "22")] {
                input.update(cx, |state, cx| {
                    state.set_value(text, window, cx);
                    cx.emit(InputEvent::Change);
                });
            }
        });
        cx.run_until_parked();
        view.update(&mut cx, |view, _| {
            let start = if remove_other { 0 } else { 1 };
            assert_eq!(view.draft.addons.mcp_servers[start].timeout_ms, Some(11));
            assert_eq!(
                view.draft.addons.mcp_servers[start + 1].timeout_ms,
                Some(22)
            );
            assert_eq!(view.editors.mcp.len(), 2);
            assert!(!view.editors.has_errors());
            assert!(
                view.has_errors(),
                "duplicate-name diagnosis must remain visible"
            );
        });
    }
}
