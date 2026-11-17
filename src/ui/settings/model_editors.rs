// Адаптировано из Zed: crates/settings_ui/src/components/input_field.rs.
//! Retained model text controls: create only for open presets, retain invalid
//! numeric text independently from the last valid typed draft.

use super::editors::EditorRevision;
use super::{SettingsView, models::model_key};
use crate::settings::models::{ModelText, edit_model_text, model_text};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use std::collections::HashMap;

pub(super) struct ModelInput {
    pub state: Entity<InputState>,
    pub error: Option<SharedString>,
    revision: EditorRevision,
    _subscription: Subscription,
}
#[derive(Default)]
pub(super) struct ModelEditors {
    pub fields: HashMap<(String, ModelText), ModelInput>,
    pub name: Option<ModelInput>,
}
impl ModelEditors {
    pub(super) fn registered(
        &mut self,
        previous: &crate::settings::Draft,
        current: &crate::settings::Draft,
        submitted: u64,
    ) {
        for ((name, field), input) in &mut self.fields {
            let old = previous
                .models
                .get(name)
                .map(|model| model_text(model, *field));
            let new = current
                .models
                .get(name)
                .map(|model| model_text(model, *field));
            input.revision.registered(submitted, old != new);
        }
    }

    pub fn has_errors(&self) -> bool {
        self.fields.values().any(|field| field.error.is_some())
    }
}
impl SettingsView {
    pub(super) fn prepare_model_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .snapshot()
            .is_none_or(|snapshot| !snapshot.models_writable)
        {
            return;
        }
        if self.editors.models.name.is_none() {
            let state =
                cx.new(|cx| InputState::new(window, cx).placeholder("Например, cheap или main"));
            let subscription =
                cx.subscribe_in(&state, window, |this, _, event, window, cx| match event {
                    InputEvent::Change => this.draft_changed(cx),
                    InputEvent::PressEnter { .. } => this.add_model_preset(window, cx),
                    _ => {}
                });
            self.editors.models.name = Some(ModelInput {
                state,
                error: None,
                revision: EditorRevision::default(),
                _subscription: subscription,
            });
        }
        let missing: Vec<_> = self
            .draft
            .models
            .iter()
            .filter(|(name, _)| self.is_open(&model_key(name)))
            .flat_map(|(name, model)| {
                ModelText::ALL
                    .into_iter()
                    .filter(|field| {
                        self.editors
                            .models
                            .fields
                            .get(&(name.clone(), *field))
                            .is_none_or(|input| input.revision.needs_refresh())
                    })
                    .map(|field| (name.clone(), field, model_text(model, field)))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (name, field, text) in missing {
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(field.placeholder())
                    .default_value(text)
            });
            let model = name.clone();
            let subscription = cx.subscribe_in(&state, window, move |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = state.read(cx).value();
                    this.model_text_changed(&model, field, &text, cx);
                }
            });
            self.editors.models.fields.insert(
                (name, field),
                ModelInput {
                    state,
                    error: None,
                    revision: EditorRevision::default(),
                    _subscription: subscription,
                },
            );
        }
    }

    pub(super) fn model_text_changed(
        &mut self,
        name: &str,
        field: ModelText,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.draft.models.get_mut(name) else {
            return;
        };
        let result = edit_model_text(model, self.baseline.models.get(name), field, text);
        if let Some(input) = self
            .editors
            .models
            .fields
            .get_mut(&(name.to_owned(), field))
        {
            input.error = result.err().map(Into::into);
            input.revision.edited(self.edit_revision.wrapping_add(1));
        }
        self.draft_changed(cx);
    }
}
