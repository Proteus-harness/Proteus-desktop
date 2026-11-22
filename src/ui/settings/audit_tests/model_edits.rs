//! Exercise the retained form handler across a delayed Save response.
use super::*;
use crate::settings::models::{ModelTarget, ModelText};

#[::core::prelude::v1::test]
fn model_form_retains_invalid_late_budget_text_until_it_is_reverted() {
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
            view.page = PageId::Models;
            view.prepare_model_editors(window, cx);
            view.model_text_changed(&ModelTarget::Root, ModelText::Model, "submitted-id", cx);
            let submitted_revision = view.edit_revision;
            let entity = view.editors.models.fields[&(ModelTarget::Root, ModelText::Budget)]
                .state
                .entity_id();
            view.model_text_changed(&ModelTarget::Root, ModelText::Budget, "invalid", cx);
            assert!(view.dirty());
            assert!(!view.can_save());
            let mut saved = snapshot("initial");
            saved.models.get_mut("initial").unwrap().model = "submitted-id".into();
            view.accept_save(saved, submitted_revision);
            let field = &view.editors.models.fields[&(ModelTarget::Root, ModelText::Budget)];
            assert_eq!(field.state.entity_id(), entity);
            assert!(field.error.is_some());
            assert!(!view.can_save());
            assert_eq!(view.draft.models["initial"].reasoning.budget_tokens, None);
            view.model_text_changed(&ModelTarget::Root, ModelText::Budget, "", cx);
            assert!(!view.dirty());
            assert!(!view.has_errors());
            assert_eq!(view.draft, view.baseline);
        })
    });
}

#[::core::prelude::v1::test]
fn model_effort_list_undo_restores_exact_source_values_without_reverting_other_fields() {
    for efforts in [
        vec!["low", " high"],
        vec!["opaque,provider-id", " strange "],
    ] {
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
                let mut saved = snapshot("initial");
                saved.models.get_mut("initial").unwrap().reasoning_efforts =
                    efforts.into_iter().map(str::to_owned).collect();
                view.accept(saved);
                view.page = PageId::Models;
                view.prepare_model_editors(window, cx);
                assert_eq!(
                    view.draft, view.baseline,
                    "Opening must not normalize opaque IDs"
                );
                let displayed = crate::settings::models::model_text(
                    &view.baseline.models["initial"],
                    ModelText::Efforts,
                );
                view.model_text_changed(&ModelTarget::Root, ModelText::Model, "local-model", cx);
                view.model_text_changed(&ModelTarget::Root, ModelText::Efforts, "low, medium", cx);
                assert_ne!(
                    view.draft.models["initial"].reasoning_efforts,
                    view.baseline.models["initial"].reasoning_efforts
                );
                view.model_text_changed(&ModelTarget::Root, ModelText::Efforts, &displayed, cx);
                assert_eq!(
                    view.draft.models["initial"].reasoning_efforts,
                    view.baseline.models["initial"].reasoning_efforts
                );
                assert_eq!(view.draft.models["initial"].model, "local-model");
                assert!(view.dirty());
                view.model_text_changed(&ModelTarget::Root, ModelText::Model, "test", cx);
                assert!(!view.dirty());
                assert!(view.draft.changes(&view.baseline).models.is_none());
            })
        });
    }
}
