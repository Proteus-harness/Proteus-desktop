use super::*;
use crate::settings::modes::{ModeList, ModeText};
use crate::ui::settings::{modes::mode_key, tags::Edit};
use proteus_contracts::domain::{BehaviorModeDefinition, PermissionMode};
use serde_json::json;

fn mode_snapshot(instructions: Option<String>) -> ConfigBuilderSnapshot {
    ConfigBuilderSnapshot {
        writable: true,
        modes_writable: true,
        modes: [(
            "review".into(),
            BehaviorModeDefinition {
                instructions,
                tools: None,
                hooks: Some(vec!["unknown-hook".into()]),
                ..Default::default()
            },
        )]
        .into(),
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
fn clean(view: &SettingsView) {
    assert!(!view.dirty());
    assert!(view.draft.areas(&view.baseline).is_empty());
    assert!(view.draft.changes(&view.baseline).modes.is_none());
}

#[::core::prelude::v1::test]
fn behavior_handlers_undo_text_and_inheritance_without_touching_siblings() {
    for instructions in [None, Some(String::new())] {
        let mut cx = context();
        let (_, view) = view(&mut cx);
        view.update(&mut cx, |view, cx| {
            view.accept(mode_snapshot(instructions));
            view.behavior_text_changed("review", ModeText::Instructions, "extra", cx);
            view.behavior_text_changed("review", ModeText::Instructions, "", cx);
            clean(view);
            view.override_behavior_list("review", ModeList::Tools, true, cx);
            assert_eq!(view.draft.modes["review"].tools, Some(vec![]));
            assert!(view.dirty());
            view.override_behavior_list("review", ModeList::Tools, false, cx);
            clean(view);
            view.override_behavior_list("review", ModeList::Hooks, false, cx);
            view.override_behavior_list("review", ModeList::Hooks, true, cx);
            clean(view);
            view.set_behavior_permission("review", Some(PermissionMode::Plan), cx);
            assert!(view.dirty());
            view.set_behavior_permission("review", None, cx);
            clean(view);
            view.update_draft(cx, |draft| draft.set_tool("sibling", true));
            view.behavior_text_changed("review", ModeText::Instructions, "extra", cx);
            view.behavior_text_changed("review", ModeText::Instructions, "", cx);
            assert_eq!(view.draft.modes, view.baseline.modes);
            assert!(view.draft.tools.contains("sibling"));
            assert!(view.dirty());
        });
    }
}

#[::core::prelude::v1::test]
fn behavior_actions_keep_unknown_tags_reject_stale_edits_and_keep_explicit_empty() {
    let mut cx = context();
    let (_, view) = view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(mode_snapshot(None));
        let expected = vec![json!("unknown-hook")];
        assert!(view.apply_behavior_tag_edit(
            "review",
            ModeList::Hooks,
            &expected,
            Edit::Add(json!("custom")),
            cx
        ));
        assert!(!view.apply_behavior_tag_edit(
            "review",
            ModeList::Hooks,
            &expected,
            Edit::Remove(0),
            cx
        ));
        assert_eq!(
            view.draft.modes["review"].hooks,
            Some(vec!["unknown-hook".into(), "custom".into()])
        );
        assert!(view.apply_behavior_tag_edit(
            "review",
            ModeList::Hooks,
            &[json!("unknown-hook"), json!("custom")],
            Edit::Remove(1),
            cx
        ));
        clean(view);
        assert!(view.apply_behavior_tag_edit(
            "review",
            ModeList::Hooks,
            &expected,
            Edit::Remove(0),
            cx
        ));
        assert_eq!(view.draft.modes["review"].hooks, Some(vec![]));
        assert!(view.draft.changes(&view.baseline).modes.is_some());
        assert!(view.draft.config.is_empty());
    });
}

#[::core::prelude::v1::test]
fn behavior_crud_handlers_roundtrip_rename_and_readonly_sources_ignore_stale_actions() {
    let mut cx = context();
    let (window, view) = view(&mut cx);
    window.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.accept(mode_snapshot(None));
            view.page = crate::settings::catalog::PageId::Modes;
            view.open.insert(mode_key("review"));
            view.prepare_mode_editors(window, cx);
            let name = view.editors.modes.name.clone().unwrap();
            name.update(cx, |input, cx| input.set_value("new", window, cx));
            view.add_behavior_mode(window, cx);
            assert!(view.draft.modes.contains_key("new"));
            view.remove_behavior_mode("new", cx);
            clean(view);
            let id = view.editors.modes.fields["review"].id.clone();
            id.update(cx, |input, cx| input.set_value("renamed", window, cx));
            view.rename_behavior_mode("review", window, cx);
            assert!(view.draft.modes.contains_key("renamed"));
            view.prepare_mode_editors(window, cx);
            let id = view.editors.modes.fields["renamed"].id.clone();
            id.update(cx, |input, cx| input.set_value("review", window, cx));
            view.rename_behavior_mode("renamed", window, cx);
            clean(view);
            let mut readonly = mode_snapshot(None);
            readonly.modes_writable = false;
            view.accept(readonly);
            view.behavior_text_changed("review", ModeText::Title, "forbidden", cx);
            view.remove_behavior_mode("review", cx);
            view.set_behavior_permission("review", Some(PermissionMode::Auto), cx);
            view.override_behavior_list("review", ModeList::Tools, true, cx);
            clean(view);
        })
    });
}

#[::core::prelude::v1::test]
fn saved_modes_acknowledge_only_the_submitted_revision_and_next_save_uses_new_cas() {
    let mut cx = context();
    let (_, view) = view(&mut cx);
    view.update(&mut cx, |view, cx| {
        view.accept(mode_snapshot(None));
        view.behavior_text_changed("review", ModeText::Title, "Submitted", cx);
        let submitted_revision = view.edit_revision;
        let mut saved = mode_snapshot(None);
        saved.modes = view.draft.modes.clone();
        view.behavior_text_changed("review", ModeText::Instructions, "newer", cx);
        view.accept_save(saved.clone(), submitted_revision);
        assert_eq!(
            view.draft.modes["review"].instructions.as_deref(),
            Some("newer")
        );
        assert!(view.dirty());
        let next = view.draft.changes(&view.baseline);
        assert_eq!(next.expected_modes, Some(saved.modes));
        assert!(next.models.is_none());
    });
}
