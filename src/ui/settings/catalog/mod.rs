//! Catalog screen state. Git and filesystem work stay off the UI thread.

mod actions;
mod apply;
mod component;
mod details;
mod download;
mod items;
mod keys;
mod overview;
mod sources;
mod toolbar;
mod view;
mod zip;

pub(in crate::ui) use keys::bind_keys;

use std::{cell::Cell, rc::Rc};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::SettingsView;
use crate::catalog::{
    Source,
    inventory::Inventory,
    schema::{Catalog, Kind},
};

pub(super) struct State {
    pub url: Entity<InputState>,
    pub branch: Entity<InputState>,
    pub search: Entity<InputState>,
    pub query: String,
    pub show: Show,
    pub kind: Option<Kind>,
    /// One catalog, or one category of it, picked on the overview.
    pub scope: Option<Scope>,
    pub loaded: Vec<(Source, Catalog)>,
    pub inventory: Inventory,
    pub inventory_generation: u64,
    pub selected: Option<(Source, String)>,
    /// A single outgoing pane stays alive only until its closing animation ends.
    pub closing: Option<(Source, String)>,
    pub readme: Option<Entity<crate::ui::extensions::content::FormattedText>>,
    pub readme_changes: Option<Subscription>,
    pub busy: bool,
    /// The entry whose install or removal holds `busy`.
    pub working: Option<(Source, String)>,
    /// Sources being downloaded; each shows up as soon as it is ready.
    pub loading: Vec<Source>,
    /// Errors of the downloads still running together.
    pub failures: Vec<String>,
    pub opened: bool,
    pub message: Option<(bool, String)>,
    /// The entry list; the arrows work while focus is on the page.
    pub focus: FocusHandle,
    pub scroll: UniformListScrollHandle,
    pub overview: ScrollHandle,
    /// Width of the list with the details as last laid out, in the units
    /// of `prefs::scaled`; 0 before the first layout.
    pub width: Rc<Cell<f32>>,
}

/// Which entries the list shows. The overview gives way to the list as
/// soon as a search, a type or a scope narrows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Show {
    Overview,
    All,
    Installed,
    Updates,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Scope {
    pub source: Source,
    pub category: Option<String>,
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> (Self, Subscription) {
        let url = cx.new(|cx| InputState::new(window, cx).placeholder("URL git-репозитория"));
        let branch = cx.new(|cx| InputState::new(window, cx).placeholder("Ветка (необязательно)"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Поиск в каталоге"));
        let changes = cx.subscribe_in(
            &search,
            window,
            |this, state, event, window, cx| match event {
                InputEvent::Change => {
                    this.catalogs.query = state.read(cx).value().to_lowercase();
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.enter_catalog_list(window, cx),
                _ => {}
            },
        );
        (
            Self {
                url,
                branch,
                search,
                query: String::new(),
                show: Show::Overview,
                kind: None,
                scope: None,
                loaded: Vec::new(),
                inventory: Inventory::default(),
                inventory_generation: 0,
                selected: None,
                closing: None,
                readme: None,
                readme_changes: None,
                busy: false,
                working: None,
                loading: Vec::new(),
                failures: Vec::new(),
                opened: false,
                message: None,
                focus: cx.focus_handle().tab_stop(true),
                scroll: UniformListScrollHandle::new(),
                overview: ScrollHandle::new(),
                width: Rc::default(),
            },
            changes,
        )
    }
}

impl SettingsView {
    /// The installed-extension manager and catalog are two views of the
    /// same package kind. Enter the catalog with a fresh, explicit filter.
    pub(in crate::ui::settings) fn open_extension_catalog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.catalogs.show = Show::All;
        self.catalogs.kind = Some(Kind::ClientExtension);
        self.catalogs.scope = None;
        self.catalogs.query.clear();
        self.catalogs.search.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.deselect_catalog_entry(cx);
        self.select_page(crate::settings::catalog::PageId::Catalog, window, cx);
    }
}

/// Agent Plugins and MCP servers live in the profile; the rest in client
/// folders.
pub(super) fn in_profile(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::AgentPlugin | Kind::McpServer | Kind::ProcessModule
    )
}
