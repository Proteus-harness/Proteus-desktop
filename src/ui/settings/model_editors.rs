// Адаптировано из Zed: crates/settings_ui/src/components/input_field.rs.
//! Keep an input attached to its agent even when the stored reference changes.

use super::editors::EditorRevision;
use super::{LoadState, SettingsView, models::model_target_spot};
use crate::settings::Draft;
use crate::settings::models::{ModelTarget, ModelText, edit_model_text, model_text};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::ConfigBuilderModelPreset;
use std::collections::HashMap;

pub(super) struct ModelInput {
    pub state: Entity<InputState>,
    pub error: Option<SharedString>,
    revision: EditorRevision,
    _subscription: Subscription,
}

#[derive(Default)]
pub(super) struct ModelEditors {
    pub fields: HashMap<(ModelTarget, ModelText), ModelInput>,
}

impl ModelEditors {
    pub(super) fn registered(&mut self, previous: &Draft, current: &Draft, submitted: u64) {
        self.fields.retain(|(target, field), input| {
            let old = target
                .configured(previous)
                .map(|model| model_text(model, *field));
            let new = target
                .configured(current)
                .map(|model| model_text(model, *field));
            input.revision.registered(submitted, old != new);
            new.is_some()
        });
    }

    pub fn has_errors(&self) -> bool {
        self.fields.values().any(|field| field.error.is_some())
    }
}

impl SettingsView {
    pub(super) fn prepare_model_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot().filter(|snapshot| snapshot.models_writable) else {
            return;
        };
        let targets = std::iter::once(ModelTarget::Root).chain(
            snapshot
                .model_roles
                .iter()
                .map(|role| ModelTarget::Role(role.name.clone())),
        );
        let mut missing = Vec::new();
        for target in targets.filter(|target| target.choice_writable(snapshot)) {
            if target != ModelTarget::Root && !self.is_open(&model_target_spot(&target)) {
                continue;
            }
            let Some(model) = target.configured(&self.draft) else {
                continue;
            };
            for field in ModelText::ALL {
                let key = (target.clone(), field);
                if self
                    .editors
                    .models
                    .fields
                    .get(&key)
                    .is_none_or(|input| input.revision.needs_refresh())
                {
                    missing.push((key, model_text(model, field)));
                }
            }
        }
        for ((target, field), text) in missing {
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(field.placeholder())
                    .default_value(text)
            });
            let edited = target.clone();
            let subscription = cx.subscribe_in(&state, window, move |this, state, event, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = state.read(cx).value();
                    this.model_text_changed(&edited, field, &text, cx);
                }
            });
            self.editors.models.fields.insert(
                (target, field),
                ModelInput {
                    state,
                    error: None,
                    revision: EditorRevision::default(),
                    _subscription: subscription,
                },
            );
        }
    }

    pub(super) fn set_target_model(
        &mut self,
        target: &ModelTarget,
        model: Option<ConfigBuilderModelPreset>,
        cx: &mut Context<Self>,
    ) {
        let LoadState::Ready(snapshot) = &self.state else {
            return;
        };
        target.set_model(&mut self.draft, &self.baseline, snapshot, model);
        if target.configured(&self.draft).is_none() {
            self.editors
                .models
                .fields
                .retain(|(key, _), _| key != target);
        }
        self.draft_changed(cx);
    }

    pub(super) fn model_text_changed(
        &mut self,
        target: &ModelTarget,
        field: ModelText,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(mut model) = target.configured(&self.draft).cloned() else {
            return;
        };
        let result = edit_model_text(&mut model, target.configured(&self.baseline), field, text);
        if let Some(input) = self.editors.models.fields.get_mut(&(target.clone(), field)) {
            input.error = result.as_ref().err().cloned().map(Into::into);
            input.revision.edited(self.edit_revision.wrapping_add(1));
        }
        if result.is_ok() {
            self.set_target_model(target, Some(model), cx);
        } else {
            self.draft_changed(cx);
        }
    }
}
