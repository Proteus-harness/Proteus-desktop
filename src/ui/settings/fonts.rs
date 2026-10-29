// Адаптировано из Zed: crates/settings_ui/src/components/font_picker.rs
//! Native searchable selectors backed by the startup font-family inventory.

use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::{IndexPath, Sizable as _};
use gpui_kit::*;

use super::SettingsView;
use crate::preferences::Preferences;
use crate::ui::{prefs, theme};

type FontSelect = SelectState<SearchableVec<SharedString>>;

pub(super) struct Fonts {
    interface: Entity<FontSelect>,
    chat: Entity<FontSelect>,
}

impl Fonts {
    pub fn new(window: &mut Window, cx: &mut Context<SettingsView>) -> (Self, Vec<Subscription>) {
        let values = prefs::current(cx).fonts.clone();
        let interface = selector(values.interface.as_deref(), window, cx);
        let chat = selector(values.chat.as_deref(), window, cx);
        let subscriptions = vec![
            follow(&interface, window, cx, |prefs, family| {
                prefs.fonts.interface = family
            }),
            follow(&chat, window, cx, |prefs, family| prefs.fonts.chat = family),
        ];
        (Self { interface, chat }, subscriptions)
    }

    pub fn interface(&self) -> AnyElement {
        control(&self.interface, "Из темы", "Шрифт интерфейса")
    }

    pub fn chat(&self) -> AnyElement {
        control(&self.chat, "Как интерфейс", "Шрифт чата")
    }
}

fn selector(
    current: Option<&str>,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
) -> Entity<FontSelect> {
    let mut families = theme::installed(cx).to_vec();
    // Retain an unavailable saved name so it remains visible and clearable.
    // Text still inherits the current native family until the font is installed.
    if let Some(current) = current
        && !families.iter().any(|family| family.as_ref() == current)
    {
        families.push(current.to_owned().into());
    }
    let selected = current.and_then(|current| {
        families
            .iter()
            .position(|family| family.as_ref() == current)
            .map(IndexPath::new)
    });
    cx.new(|cx| {
        SelectState::new(SearchableVec::new(families), selected, window, cx).searchable(true)
    })
}

fn follow(
    state: &Entity<FontSelect>,
    window: &mut Window,
    cx: &mut Context<SettingsView>,
    apply: fn(&mut Preferences, Option<String>),
) -> Subscription {
    cx.subscribe_in(
        state,
        window,
        move |_, _, event: &SelectEvent<SearchableVec<SharedString>>, _, cx| {
            let SelectEvent::Confirm(value) = event;
            let value = value.as_ref().map(ToString::to_string);
            prefs::update(cx, |prefs| apply(prefs, value));
            cx.notify();
        },
    )
}

fn control(state: &Entity<FontSelect>, inherited: &'static str, label: &'static str) -> AnyElement {
    Select::new(state)
        .small()
        .cleanable(true)
        .placeholder(inherited)
        .accessibility_label(label)
        .search_placeholder("Поиск шрифтов…")
        .menu_width(prefs::scaled(300.))
        .w(prefs::scaled(240.))
        .max_w_full()
        .into_any_element()
}
