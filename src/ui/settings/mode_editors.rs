// Адаптировано из Zed: crates/settings_ui/src/components/input_field.rs.
//! Create behavior-mode inputs only for expanded records. Closing settings or
//! replacing the draft drops subscriptions; no provider/model work occurs.

use std::collections::HashMap;

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;

use super::editors::EditorRevision;
use super::{SettingsView, modes::mode_key};
use crate::settings::modes::{self, ModeList, ModeText};

pub(super) struct ModeFields {
    pub id: Entity<InputState>,
    pub title: Entity<InputState>,
    pub instructions: Entity<TextareaState>,
    pub queries: HashMap<ModeList, Entity<InputState>>,
    title_revision: EditorRevision,
    instructions_revision: EditorRevision,
    _title_subscription: Subscription,
    _instructions_subscription: Subscription,
    _subscriptions: Vec<Subscription>,
}
#[derive(Default)]
pub(super) struct ModeEditors {
    pub name: Option<Entity<InputState>>,
    pub fields: HashMap<String, ModeFields>,
    _name_subscription: Option<Subscription>,
}

impl ModeEditors {
    pub(super) fn registered(
        &mut self,
        previous: &crate::settings::Draft,
        current: &crate::settings::Draft,
        submitted: u64,
    ) {
        for (name, fields) in &mut self.fields {
            let old = previous.modes.get(name);
            let new = current.modes.get(name);
            fields.title_revision.registered(
                submitted,
                old.and_then(|mode| mode.title.as_ref())
                    != new.and_then(|mode| mode.title.as_ref()),
            );
            fields.instructions_revision.registered(
                submitted,
                old.and_then(|mode| mode.instructions.as_ref())
                    != new.and_then(|mode| mode.instructions.as_ref()),
            );
        }
    }
}

impl SettingsView {
    pub(super) fn prepare_mode_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.modes_writable() {
            return;
        }
        if self.editors.modes.name.is_none() {
            let input = cx.new(|cx| InputState::new(window, cx));
            let subscription =
                cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                    InputEvent::Change => this.draft_changed(cx),
                    InputEvent::PressEnter { .. } => this.add_behavior_mode(window, cx),
                    _ => {}
                });
            self.editors.modes.name = Some(input);
            self.editors.modes._name_subscription = Some(subscription);
        }
        let refresh = self
            .editors
            .modes
            .fields
            .iter()
            .flat_map(|(name, fields)| {
                let mode = self.draft.modes.get(name);
                [
                    (ModeText::Title, fields.title_revision.needs_refresh()),
                    (
                        ModeText::Instructions,
                        fields.instructions_revision.needs_refresh(),
                    ),
                ]
                .into_iter()
                .filter(|(_, refresh)| *refresh)
                .filter_map(move |(field, _)| {
                    mode.map(|mode| (name.clone(), field, modes::text(mode, field).to_owned()))
                })
            })
            .collect::<Vec<_>>();
        for (name, field, text) in refresh {
            match field {
                ModeText::Title => {
                    let (input, subscription) = self.mode_title_input(&name, text, window, cx);
                    let fields = self.editors.modes.fields.get_mut(&name).unwrap();
                    fields.title = input;
                    fields._title_subscription = subscription;
                    fields.title_revision = EditorRevision::default();
                }
                ModeText::Instructions => {
                    let (input, subscription) =
                        self.mode_instructions_input(&name, text, window, cx);
                    let fields = self.editors.modes.fields.get_mut(&name).unwrap();
                    fields.instructions = input;
                    fields._instructions_subscription = subscription;
                    fields.instructions_revision = EditorRevision::default();
                }
            }
        }
        let missing = self
            .draft
            .modes
            .iter()
            .filter(|(name, _)| {
                self.is_open(&mode_key(name)) && !self.editors.modes.fields.contains_key(*name)
            })
            .map(|(name, mode)| {
                (
                    name.clone(),
                    modes::text(mode, ModeText::Title).to_owned(),
                    modes::text(mode, ModeText::Instructions).to_owned(),
                )
            })
            .collect::<Vec<_>>();
        for (name, title, instructions) in missing {
            let id = cx.new(|cx| InputState::new(window, cx).default_value(name.clone()));
            let target = name.clone();
            let id_change =
                cx.subscribe_in(&id, window, move |this, _, event, window, cx| match event {
                    InputEvent::Change => this.draft_changed(cx),
                    InputEvent::PressEnter { .. } => this.rename_behavior_mode(&target, window, cx),
                    _ => {}
                });
            let (title, title_change) = self.mode_title_input(&name, title, window, cx);
            let (instructions, instructions_change) =
                self.mode_instructions_input(&name, instructions, window, cx);
            let mut subscriptions = vec![id_change];
            let mut queries = HashMap::new();
            for kind in [ModeList::Tools, ModeList::Hooks] {
                let input = cx.new(|cx| InputState::new(window, cx));
                let target = name.clone();
                subscriptions.push(cx.subscribe_in(
                    &input,
                    window,
                    move |this, _, event, window, cx| match event {
                        InputEvent::Change => this.draft_changed(cx),
                        InputEvent::PressEnter { .. } => {
                            this.add_behavior_tag(&target, kind, window, cx)
                        }
                        _ => {}
                    },
                ));
                queries.insert(kind, input);
            }
            self.editors.modes.fields.insert(
                name,
                ModeFields {
                    id,
                    title,
                    instructions,
                    queries,
                    title_revision: EditorRevision::default(),
                    instructions_revision: EditorRevision::default(),
                    _title_subscription: title_change,
                    _instructions_subscription: instructions_change,
                    _subscriptions: subscriptions,
                },
            );
        }
    }

    fn mode_title_input(
        &mut self,
        name: &str,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InputState>, Subscription) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        let target = name.to_owned();
        let subscription = cx.subscribe_in(&input, window, move |this, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.behavior_text_changed(&target, ModeText::Title, &state.read(cx).value(), cx);
            }
        });
        (input, subscription)
    }

    fn mode_instructions_input(
        &mut self,
        name: &str,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<TextareaState>, Subscription) {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, super::editors::editor_max_rows(&text))
                .default_value(text)
        });
        let target = name.to_owned();
        let subscription = cx.subscribe_in(&input, window, move |this, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value();
                this.grow_editor(&state, &text, cx);
                this.behavior_text_changed(&target, ModeText::Instructions, &text, cx);
            }
        });
        (input, subscription)
    }

    pub(super) fn behavior_text_changed(
        &mut self,
        id: &str,
        field: ModeText,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        if !self.modes_writable() {
            return;
        }
        let Some(mode) = self.draft.modes.get_mut(id) else {
            return;
        };
        modes::set_text(mode, self.baseline.modes.get(id), field, text);
        if let Some(fields) = self.editors.modes.fields.get_mut(id) {
            let revision = match field {
                ModeText::Title => &mut fields.title_revision,
                ModeText::Instructions => &mut fields.instructions_revision,
            };
            revision.edited(self.edit_revision.wrapping_add(1));
        }
        self.draft_changed(cx);
    }
}
