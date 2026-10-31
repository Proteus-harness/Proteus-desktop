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

    pub fn reset_interface(&self, window: &mut Window, cx: &mut Context<SettingsView>) {
        self.interface.update(cx, |state, cx| {
            state.set_selected_index(None, window, cx);
            cx.notify();
        });
    }

    pub fn reset_chat(&self, window: &mut Window, cx: &mut Context<SettingsView>) {
        self.chat.update(cx, |state, cx| {
            state.set_selected_index(None, window, cx);
            cx.notify();
        });
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

#[derive(IntoElement)]
struct FontControl {
    state: Entity<FontSelect>,
    inherited: &'static str,
    label: &'static str,
}

impl RenderOnce for FontControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let value = self
            .state
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| self.inherited.into());
        let style = window.text_style();
        let font_size = prefs::scaled(14.).to_pixels(window.rem_size());
        let width = window
            .text_system()
            .shape_line(value.clone(), font_size, &[style.to_run(value.len())], None)
            .width();
        div()
            .min_w_0()
            .max_w_full()
            .w(width + prefs::scaled(56.).to_pixels(window.rem_size()))
            .child(
                Select::new(&self.state)
                    .small()
                    .cleanable(true)
                    .placeholder(self.inherited)
                    .accessibility_label(self.label)
                    .search_placeholder("Поиск шрифтов…")
                    .render_title(|title, _, _| crate::ui::fade::faded(title).into_any_element())
                    .menu_width(prefs::scaled(300.))
                    .min_w_0()
                    .w_full(),
            )
    }
}

fn control(state: &Entity<FontSelect>, inherited: &'static str, label: &'static str) -> AnyElement {
    FontControl {
        state: state.clone(),
        inherited,
        label,
    }
    .into_any_element()
}
