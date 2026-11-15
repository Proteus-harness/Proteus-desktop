//! Controlled completion ordering through the real settings methods.
//! These do not dispatch HTTP requests or run a profile/runtime.

use super::*;
use crate::settings::reconcile::CatalogChange;
use serde_json::json;

fn new_view(
    cx: &mut ::gpui::TestAppContext,
) -> (&mut ::gpui::VisualTestContext, Entity<SettingsView>) {
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

fn pending_read(view: &mut SettingsView) -> (u64, u64) {
    assert!(!view.reloading);
    view.reloading = true;
    (view.profile_revision, view.edit_revision)
}

fn module_snapshot(a: i64, b: i64) -> ConfigBuilderSnapshot {
    let mut result = snapshot("initial");
    result.module_config.insert(
        "tool".into(),
        std::collections::BTreeMap::from([("files".into(), json!({"a": a, "b": b}))]),
    );
    result
}

#[::core::prelude::v1::test]
fn delayed_reload_cannot_revert_a_saved_value_in_the_next_save_request() {
    let mut cx = context();
    let (_, view) = new_view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(module_snapshot(1, 1));
        let (profile_revision, edit_revision) = pending_read(view);
        view.update_draft(cx, |draft| {
            draft
                .config
                .get_mut("tool")
                .unwrap()
                .get_mut("files")
                .unwrap()["a"] = json!(2);
        });
        assert!(view.can_save(), "Save stays available during the read");
        view.accept_save(module_snapshot(2, 1), view.edit_revision);
        assert!(!view.dirty());

        view.finish_load(
            Ok(module_snapshot(1, 1)),
            profile_revision,
            edit_revision,
            cx,
        );

        assert!(!view.reloading);
        assert!(!view.dirty());
        view.update_draft(cx, |draft| {
            draft
                .config
                .get_mut("tool")
                .unwrap()
                .get_mut("files")
                .unwrap()["b"] = json!(2);
        });
        let request = view.draft.changes(&view.baseline);
        assert_eq!(
            request.module_config["tool"]["files"],
            json!({"a": 2, "b": 2})
        );
    });
}

#[::core::prelude::v1::test]
fn catalog_ack_invalidates_old_reads_without_reclassifying_local_edits() {
    let mut cx = context();
    let (_, view) = new_view(&mut cx);
    view.update(&mut cx, |view, cx| {
        let initial = snapshot("initial");
        view.accept(initial.clone());
        let (profile_revision, edit_revision) = pending_read(view);
        let change = CatalogChange {
            before: view.baseline.addons.clone(),
            replacement: None,
        };
        let mut accepted = initial.clone();
        accepted.addon_settings = serde_json::from_value(json!({
            "addons": {"plugins": [{"path": "catalog"}]}, "mcp_servers": []
        }))
        .unwrap();
        view.accept_catalog_save(accepted, edit_revision, &change);
        assert_eq!(view.edit_revision, edit_revision);
        assert_ne!(view.profile_revision, profile_revision);

        view.finish_load(Ok(initial), profile_revision, edit_revision, cx);

        assert_eq!(view.draft.addons.addons.plugins.len(), 1);
        assert_eq!(
            view.draft.addons.addons.plugins[0].path,
            std::path::Path::new("catalog")
        );
        assert!(!view.dirty());
    });
}

#[::core::prelude::v1::test]
fn equal_value_acks_advance_the_profile_revision_in_both_ack_branches() {
    for newer_edit in [false, true] {
        let mut cx = context();
        let (_, view) = new_view(&mut cx);
        view.update(&mut cx, |view, cx| {
            view.accept(snapshot("initial"));
            let (profile_revision, edit_revision) = pending_read(view);
            if newer_edit {
                view.update_draft(cx, |draft| draft.set_tool("newer-tool", true));
            }
            let current_edit_revision = view.edit_revision;
            view.accept_save(snapshot("initial"), edit_revision);
            assert_eq!(view.edit_revision, current_edit_revision);
            assert_ne!(view.profile_revision, profile_revision);
            view.feedback = Some(Feedback {
                text: "accepted".into(),
                error: false,
            });

            view.finish_load(
                Err(anyhow::anyhow!("old read failed")),
                profile_revision,
                edit_revision,
                cx,
            );

            assert_eq!(view.feedback.as_ref().unwrap().text.as_ref(), "accepted");
            assert!(!view.feedback.as_ref().unwrap().error);
            assert_eq!(view.draft.tools.contains("newer-tool"), newer_edit);
            assert_eq!(view.dirty(), newer_edit);
            assert!(!view.reloading);
        });
    }
}

#[::core::prelude::v1::test]
fn current_read_success_and_errors_keep_their_existing_behavior() {
    let mut cx = context();
    let (_, view) = new_view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(snapshot("initial"));
        let (profile_revision, edit_revision) = pending_read(view);
        view.finish_load(Ok(snapshot("fresh")), profile_revision, edit_revision, cx);
        assert_eq!(view.baseline.provider.as_deref(), Some("fresh"));
        assert_eq!(view.draft.provider, view.baseline.provider);
        assert_ne!(view.profile_revision, profile_revision);
        assert!(!view.reloading);
        let (profile_revision, edit_revision) = pending_read(view);
        view.finish_load(
            Err(anyhow::anyhow!("current read failed")),
            profile_revision,
            edit_revision,
            cx,
        );
        assert_eq!(view.baseline.provider.as_deref(), Some("fresh"));
        assert!(view.feedback.as_ref().unwrap().error);
    });
    let (_, empty) = new_view(&mut cx);
    empty.update(&mut cx, |view, cx| {
        let (profile_revision, edit_revision) = pending_read(view);
        view.finish_load(
            Err(anyhow::anyhow!("initial read failed")),
            profile_revision,
            edit_revision,
            cx,
        );
        assert!(matches!(&view.state, LoadState::Failed(_)));
        assert!(!view.reloading);
    });
}

#[::core::prelude::v1::test]
fn newer_raw_buffers_survive_reload_even_without_a_dirty_parsed_draft() {
    for raw in ["{  }", "{invalid"] {
        let mut cx = context();
        let (window, view) = new_view(&mut cx);
        window.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.accept(snapshot("initial"));
                view.ensure_json("tool", "synthetic", window, cx);
                let key = ("tool".into(), "synthetic".into());
                let input = view.editors.json[&key].input.clone();
                let (profile_revision, edit_revision) = pending_read(view);
                input.update(cx, |input, cx| input.set_value(raw, window, cx));
                view.json_changed(&key, raw, cx);
                assert_eq!(view.dirty(), raw == "{invalid");

                view.finish_load(
                    Ok(snapshot("old read")),
                    profile_revision,
                    edit_revision,
                    cx,
                );

                assert_eq!(view.editors.json[&key].input.entity_id(), input.entity_id());
                assert_eq!(input.read(cx).value().as_ref(), raw);
                assert_eq!(view.draft.provider.as_deref(), Some("initial"));
                assert_eq!(view.profile_revision, profile_revision);
            });
        });
    }
}

#[::core::prelude::v1::test]
fn edit_then_undo_does_not_authorize_an_older_read() {
    let mut cx = context();
    let (_, view) = new_view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(snapshot("initial"));
        let (profile_revision, edit_revision) = pending_read(view);
        view.update_draft(cx, |draft| draft.provider = Some("temporary".into()));
        view.discard(cx);
        assert!(!view.dirty());
        view.finish_load(
            Ok(snapshot("old read")),
            profile_revision,
            edit_revision,
            cx,
        );
        assert_eq!(view.baseline.provider.as_deref(), Some("initial"));
        assert!(!view.dirty());
    });
}

#[::core::prelude::v1::test]
fn a_read_started_already_dirty_retains_the_existing_dirty_guard() {
    let mut cx = context();
    let (_, view) = new_view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(snapshot("initial"));
        view.update_draft(cx, |draft| draft.provider = Some("local".into()));
        let (profile_revision, edit_revision) = pending_read(view);
        view.finish_load(
            Ok(snapshot("old read")),
            profile_revision,
            edit_revision,
            cx,
        );
        assert_eq!(view.draft.provider.as_deref(), Some("local"));
        assert!(view.dirty());
    });
}

#[::core::prelude::v1::test]
fn active_read_rejects_another_load_before_changing_view_state() {
    let mut cx = context();
    let (_, view) = new_view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(snapshot("initial"));
        let (profile_revision, edit_revision) = pending_read(view);
        // This view has no HTTP context. The early in-flight guard must return
        // even before the unavailable-context branch changes the visible state.
        view.load(cx);
        assert!(view.reloading);
        assert_eq!(
            view.snapshot().unwrap().active_provider.as_deref(),
            Some("initial")
        );
        assert_eq!(
            (view.profile_revision, view.edit_revision),
            (profile_revision, edit_revision)
        );
    });
}
