//! Text editors of the profile MCP server forms. They are kept per position
//! in `tools.mcp_servers`, built for open servers only and rebuilt from the
//! draft when the list shifts.

use std::{cell::Cell, collections::HashMap, rc::Rc};

use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use proteus_contracts::domain::ConfiguredMcpServerConfig;
use proteus_contracts::domain::mcp_protocol_version;

use super::SettingsView;
use super::editors::{TextInput, editor_max_rows};
use super::mcp::{mcp_advanced_key, mcp_key};
use crate::settings::mcp::{McpText, mcp_text};

pub(super) struct McpForm {
    pub inputs: HashMap<McpText, TextInput>,
    /// Text that does not parse; the draft keeps the last valid value.
    pub errors: HashMap<McpText, SharedString>,
    pub edited: bool,
    index: Rc<Cell<usize>>,
    _subscriptions: Vec<Subscription>,
}

impl McpForm {
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

fn placeholder(field: McpText) -> String {
    match field {
        McpText::Name => "Имя сервера".into(),
        McpText::Command => "Например, npx".into(),
        McpText::Args => "Аргумент на строку".into(),
        McpText::Cwd => "Рабочая папка агента".into(),
        McpText::Env => "KEY=VALUE".into(),
        McpText::EnvAllowlist => "Имя переменной на строку".into(),
        McpText::TimeoutMs | McpText::MaxResponseBytes => "По умолчанию".into(),
        McpText::ProtocolVersion => mcp_protocol_version(),
    }
}

impl SettingsView {
    /// Builds editors for the open profile servers that lack them.
    pub(super) fn prepare_mcp_forms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let missing: Vec<usize> = self
            .draft
            .addons
            .mcp_servers
            .iter()
            .enumerate()
            .filter(|(ix, server)| {
                !self.editors.mcp.contains_key(ix) && self.is_open(&mcp_key(&server.name))
            })
            .map(|(ix, _)| ix)
            .collect();
        for ix in missing {
            let form = self.mcp_form_editors(ix, window, cx);
            self.editors.mcp.insert(ix, form);
        }
    }

    fn mcp_form_editors(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> McpForm {
        let server = &self.draft.addons.mcp_servers[ix];
        let mut inputs = HashMap::new();
        let mut subscriptions = Vec::new();
        let index = Rc::new(Cell::new(ix));
        for field in McpText::ALL {
            let (text, hint) = (mcp_text(server, field), placeholder(field));
            let index = index.clone();
            let (input, subscription) = if field.is_multiline() {
                let state = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(2, editor_max_rows(&text))
                        .placeholder(hint)
                        .default_value(text)
                });
                let subscription =
                    cx.subscribe_in(&state, window, move |this, state, event, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            let text = state.read(cx).value();
                            this.grow_editor(&state, &text, cx);
                            this.mcp_text_changed(index.get(), field, &text, cx);
                        }
                    });
                (TextInput::Area(state), subscription)
            } else {
                let state = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(hint)
                        .default_value(text)
                });
                let subscription =
                    cx.subscribe_in(&state, window, move |this, state, event, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            let text = state.read(cx).value();
                            this.mcp_text_changed(index.get(), field, &text, cx);
                        }
                    });
                (TextInput::Line(state), subscription)
            };
            inputs.insert(field, input);
            subscriptions.push(subscription);
        }
        McpForm {
            inputs,
            errors: HashMap::new(),
            edited: false,
            index,
            _subscriptions: subscriptions,
        }
    }

    fn mcp_name(&self, ix: usize) -> Option<String> {
        self.draft
            .addons
            .mcp_servers
            .get(ix)
            .map(|server| server.name.clone())
    }

    /// A rename carries the open disclosures of the server with it.
    pub(super) fn mcp_text_changed(
        &mut self,
        ix: usize,
        field: McpText,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let old = self.mcp_name(ix);
        let mut result: Result<(), String> = Ok(());
        self.update_draft(cx, |draft| result = draft.set_mcp_text(ix, field, text));
        if let Some(form) = self.editors.mcp.get_mut(&ix) {
            form.edited = true;
            match result {
                Ok(()) => form.errors.remove(&field),
                Err(error) => form.errors.insert(field, error.into()),
            };
        }
        if let (Some(old), Some(new)) = (old, self.mcp_name(ix))
            && old != new
        {
            for (from, to) in [
                (mcp_key(&old), mcp_key(&new)),
                (mcp_advanced_key(&old), mcp_advanced_key(&new)),
            ] {
                if self.open.remove(&from) {
                    self.open.insert(to);
                }
            }
        }
    }

    /// Rebind retained callbacks when a catalog ACK changes list positions.
    /// Untouched forms for changed/removed servers can be rebuilt; edited
    /// forms keep their exact input entities, including invalid raw text.
    pub(super) fn remap_mcp_editors(&mut self, before: &[ConfiguredMcpServerConfig]) {
        let after = &self.draft.addons.mcp_servers;
        if before == after {
            return;
        }
        // A partially edited draft may contain duplicate names, even two
        // identical parsed entries with different raw input buffers. Match
        // every old row in order and claim each destination at most once;
        // iterating only the open forms would misidentify an unopened twin.
        let mut claimed = vec![false; after.len()];
        let positions: Vec<_> = before
            .iter()
            .map(|server| {
                let next = after
                    .iter()
                    .enumerate()
                    .find_map(|(ix, entry)| (!claimed[ix] && entry == server).then_some(ix))?;
                claimed[next] = true;
                Some(next)
            })
            .collect();
        self.editors.mcp = std::mem::take(&mut self.editors.mcp)
            .into_iter()
            .filter_map(|(ix, form)| {
                let next = *positions.get(ix)?.as_ref()?;
                form.index.set(next);
                Some((next, form))
            })
            .collect();
    }

    /// Positions shift, so every form is rebuilt from the draft.
    pub(super) fn remove_mcp_server(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(name) = self.mcp_name(ix) {
            self.open.remove(&mcp_key(&name));
            self.open.remove(&mcp_advanced_key(&name));
        }
        self.editors.mcp.clear();
        self.update_draft(cx, |draft| draft.remove_mcp(ix));
    }
}
