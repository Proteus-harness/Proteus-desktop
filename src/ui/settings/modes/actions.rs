//! Draft-only mode actions; every handler checks current source ownership.

use super::{SettingsView, mode_key};
use crate::settings::modes::ModeList;
use crate::ui::settings::tags::{self, Edit};
use gpui_kit::*;
use proteus_contracts::domain::PermissionMode;
use serde_json::Value;

impl SettingsView {
    pub(in crate::ui::settings) fn set_behavior_permission(
        &mut self,
        id: &str,
        value: Option<PermissionMode>,
        cx: &mut Context<Self>,
    ) {
        if self.modes_writable()
            && value.as_ref().is_none_or(|value| {
                self.snapshot().is_some_and(|snapshot| {
                    snapshot
                        .permission_modes
                        .iter()
                        .any(|id| id == value.as_str())
                })
            })
            && let Some(mode) = self.draft.modes.get_mut(id)
        {
            mode.permission_mode = value;
            self.draft_changed(cx);
        }
    }
    pub(in crate::ui::settings) fn add_behavior_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.modes_writable() {
            return;
        }
        let Some(input) = self.editors.modes.name.clone() else {
            return;
        };
        let name = input.read(cx).value().to_string();
        if self.draft.add_mode(&name).is_ok() {
            self.open.insert(mode_key(&name));
            input.update(cx, |input, cx| input.set_value("", window, cx));
            self.draft_changed(cx);
        }
    }
    pub(in crate::ui::settings) fn rename_behavior_mode(
        &mut self,
        id: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.modes_writable() {
            return;
        }
        let Some(fields) = self.editors.modes.fields.get(id) else {
            return;
        };
        let name = fields.id.read(cx).value().to_string();
        if self.draft.rename_mode(id, &name).is_ok() && name != id {
            self.editors.modes.fields.remove(id);
            self.open.remove(&mode_key(id));
            self.open.insert(mode_key(&name));
            self.draft_changed(cx);
        }
    }
    pub(in crate::ui::settings) fn remove_behavior_mode(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.modes_writable() && self.draft.modes.remove(id).is_some() {
            self.editors.modes.fields.remove(id);
            self.open.remove(&mode_key(id));
            self.draft_changed(cx);
        }
    }
    pub(in crate::ui::settings) fn override_behavior_list(
        &mut self,
        id: &str,
        kind: ModeList,
        override_list: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.modes_writable() {
            return;
        }
        let inherited = override_list.then(|| {
            self.baseline
                .modes
                .get(id)
                .and_then(|mode| kind.values(mode))
                .cloned()
                .unwrap_or_else(|| kind.baseline(&self.draft))
        });
        let Some(mode) = self.draft.modes.get_mut(id) else {
            return;
        };
        if override_list && kind.values(mode).is_some() {
            return;
        }
        kind.set(mode, inherited);
        self.draft_changed(cx);
    }
    pub(in crate::ui::settings) fn apply_behavior_tag_edit(
        &mut self,
        id: &str,
        kind: ModeList,
        expected: &[Value],
        edit: Edit,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.modes_writable() {
            return false;
        }
        let Some(mode) = self.draft.modes.get_mut(id) else {
            return false;
        };
        let Some(current) = kind.values(mode) else {
            return false;
        };
        if current.len() != expected.len()
            || !current
                .iter()
                .zip(expected)
                .all(|(left, right)| right.as_str() == Some(left.as_str()))
        {
            return false;
        }
        let Some(next) = tags::edited(expected, edit) else {
            return false;
        };
        let Some(next) = next
            .iter()
            .map(|value| value.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
        else {
            return false;
        };
        kind.set(mode, Some(next));
        self.draft_changed(cx);
        true
    }
    pub(in crate::ui::settings) fn add_behavior_tag(
        &mut self,
        id: &str,
        kind: ModeList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(input) = self
            .editors
            .modes
            .fields
            .get(id)
            .and_then(|fields| fields.queries.get(&kind))
            .cloned()
        else {
            return;
        };
        let text = input.read(cx).value().to_string();
        if text.is_empty() {
            return;
        }
        let Some(values) = self.draft.modes.get(id).and_then(|mode| kind.values(mode)) else {
            return;
        };
        let expected = values
            .iter()
            .cloned()
            .map(Value::String)
            .collect::<Vec<_>>();
        if self.apply_behavior_tag_edit(id, kind, &expected, Edit::Add(Value::String(text)), cx) {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }
}
