// Адаптировано из Zed: crates/settings_ui/src/components/font_picker.rs
//! Font selectors reuse the application's searchable picker and reversible
//! popup. The installed-family inventory is collected once at startup.

use std::{cell::Cell, rc::Rc};

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SettingsView;
use crate::preferences::Preferences;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::picker::{Choice, Picker};
use crate::ui::{kit, motion, prefs, theme};

type FontPicker = Picker<Option<SharedString>>;

#[derive(Clone, Copy)]
enum Target {
    Interface,
    Chat,
}

impl Target {
    fn value(self, prefs: &Preferences) -> Option<&str> {
        match self {
            Self::Interface => prefs.fonts.interface.as_deref(),
            Self::Chat => prefs.fonts.chat.as_deref(),
        }
    }

    fn apply(self, prefs: &mut Preferences, value: Option<String>) {
        match self {
            Self::Interface => prefs.fonts.interface = value,
            Self::Chat => prefs.fonts.chat = value,
        }
    }

    fn inherited(self) -> &'static str {
        match self {
            Self::Interface => "Из темы",
            Self::Chat => "Как интерфейс",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Interface => "Шрифт интерфейса",
            Self::Chat => "Шрифт чата",
        }
    }
}

pub(super) struct Fonts {
    interface: Entity<FontControl>,
    chat: Entity<FontControl>,
}

impl Fonts {
    pub fn new(_: &mut Window, cx: &mut Context<SettingsView>) -> (Self, Vec<Subscription>) {
        let interface = cx.new(|cx| FontControl::new(Target::Interface, cx));
        let chat = cx.new(|cx| FontControl::new(Target::Chat, cx));
        let changes = vec![
            cx.subscribe(&interface, |_, _, _: &FontChanged, cx| cx.notify()),
            cx.subscribe(&chat, |_, _, _: &FontChanged, cx| cx.notify()),
        ];
        (Self { interface, chat }, changes)
    }

    pub fn reset_interface(&self, window: &mut Window, cx: &mut Context<SettingsView>) {
        self.interface.update(cx, |state, cx| {
            state.close(window, cx);
            cx.notify();
        });
    }

    pub fn reset_chat(&self, window: &mut Window, cx: &mut Context<SettingsView>) {
        self.chat.update(cx, |state, cx| {
            state.close(window, cx);
            cx.notify();
        });
    }

    pub fn close(&self, window: &mut Window, cx: &mut Context<SettingsView>) {
        self.interface
            .update(cx, |state, cx| state.close(window, cx));
        self.chat.update(cx, |state, cx| state.close(window, cx));
    }

    pub fn interface(&self) -> AnyElement {
        self.interface.clone().into_any_element()
    }

    pub fn chat(&self) -> AnyElement {
        self.chat.clone().into_any_element()
    }
}

/// Keep a missing saved family visible and clearable without rediscovering fonts.
fn choices(
    families: &[SharedString],
    current: Option<&str>,
    inherited: &str,
) -> Vec<Choice<Option<SharedString>>> {
    let mut options = Vec::with_capacity(families.len() + 2);
    options.push(Choice {
        label: inherited.to_owned().into(),
        value: None,
        selected: current.is_none(),
    });
    options.extend(families.iter().map(|family| Choice {
        label: family.clone(),
        value: Some(family.clone()),
        selected: current == Some(family.as_ref()),
    }));
    if let Some(current) = current
        && !families.iter().any(|family| family.as_ref() == current)
    {
        let family: SharedString = current.to_owned().into();
        options.push(Choice {
            label: family.clone(),
            value: Some(family),
            selected: true,
        });
    }
    options
}

#[derive(Clone)]
struct FontPopup {
    picker: Entity<FontPicker>,
    focus: FocusHandle,
}

struct FontControl {
    target: Target,
    focus: FocusHandle,
    popup: motion::Popup<FontPopup>,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    pressed_open: bool,
}

struct FontChanged;
impl EventEmitter<FontChanged> for FontControl {}

impl FontControl {
    fn new(target: Target, cx: &mut Context<Self>) -> Self {
        Self {
            target,
            focus: cx.focus_handle(),
            popup: motion::Popup::default(),
            bounds: Rc::default(),
            pressed_open: false,
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.popup.current().cloned() else {
            return;
        };
        self.popup.close();
        motion::restore_focus(&popup.focus, Some(self.focus.clone()), window, cx);
        cx.notify();
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pressed_open = std::mem::take(&mut self.pressed_open);
        if self.popup.is_open() || (pressed_open && !window.last_input_was_keyboard()) {
            self.close(window, cx);
            return;
        }
        let options = choices(
            theme::installed(cx),
            self.target.value(prefs::current(cx)),
            self.target.inherited(),
        );
        let owner = cx.weak_entity();
        let picker = cx.new(|cx| {
            Picker::new(
                options,
                "Поиск шрифтов…".into(),
                "Шрифты не найдены".into(),
                window,
                cx,
                move |value: Option<SharedString>, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        let value = value.map(|value| value.to_string());
                        prefs::update(cx, |prefs| this.target.apply(prefs, value));
                        this.close(window, cx);
                        // The settings page owns the reset marker and save-error
                        // notice; refresh it after this child commits a choice.
                        cx.emit(FontChanged);
                    });
                },
            )
        });
        let focus = picker.read(cx).focus(cx);
        focus.focus(window, cx);
        self.popup.show(FontPopup { picker, focus });
        cx.notify();
    }
}

impl Render for FontControl {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        motion::drive(window, cx);
        let value: SharedString = self
            .target
            .value(prefs::current(cx))
            .unwrap_or(self.target.inherited())
            .to_owned()
            .into();
        let style = window.text_style();
        let font_size = prefs::scaled(14.).to_pixels(window.rem_size());
        let width = window
            .text_system()
            .shape_line(value.clone(), font_size, &[style.to_run(value.len())], None)
            .width();
        let measured = self.bounds.clone();
        let open = self.popup.is_open();
        let root = div()
            .id("font-selector")
            .relative()
            .min_w_0()
            .max_w_full()
            .w(width + prefs::scaled(32.).to_pixels(window.rem_size()))
            .capture_any_mouse_down(cx.listener(move |this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Left {
                    this.pressed_open = open;
                }
            }))
            .child(
                kit::button("font-trigger")
                    .track_focus(&self.focus)
                    .small()
                    .faded_label(value)
                    .accessibility_label(self.target.label())
                    .px_1p5()
                    .outline()
                    .min_w_0()
                    .w_full()
                    .child(motion::menu_chevron(
                        "font-caret",
                        open,
                        cx.theme().muted_foreground,
                    ))
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.toggle(window, cx);
                    })),
            )
            .child(
                canvas(move |bounds, _, _| measured.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            );
        let Some(frame) = self.popup.frame(self.bounds.get(), window, cx) else {
            return root;
        };
        let popup = self.popup.value().unwrap().clone();
        let focus = popup.focus.clone();
        let generation = self.popup.generation();
        let trigger = self.bounds.clone();
        let owner = cx.weak_entity();
        let room = window.viewport_size();
        let width = prefs::scaled(300.)
            .to_pixels(window.rem_size())
            .min((room.width - prefs::scaled(16.).to_pixels(window.rem_size())).max(px(0.)));
        let body = v_flex()
            .relative()
            .id("font-popup")
            .track_focus(&focus)
            .w(width)
            .p_1p5()
            .child(popup.picker)
            .when(frame.open, |body| {
                body.on_action(cx.listener(
                    |this, _: &crate::ui::FocusSettingsSearch, window, cx| {
                        if let Some(popup) = this.popup.current() {
                            popup.picker.read(cx).search_focus(cx).focus(window, cx);
                            window.refresh();
                        }
                        cx.stop_propagation();
                    },
                ))
                .on_action(
                    cx.listener(|this, _: &crate::ui::DismissSettings, window, cx| {
                        this.close(window, cx);
                        cx.stop_propagation();
                    }),
                )
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "tab" {
                        if let Some(popup) = this.popup.current() {
                            kit::cycle_focus(
                                &popup.focus,
                                !event.keystroke.modifiers.shift,
                                window,
                                cx,
                            );
                        }
                        cx.stop_propagation();
                    }
                }))
                .on_mouse_down_out(move |event, window, cx| {
                    if !trigger.get().contains(&event.position) {
                        let owner = owner.clone();
                        window.defer(cx, move |window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if this.popup.is_current(generation) {
                                    this.close(window, cx);
                                }
                            });
                        });
                    }
                })
            });
        let body = if frame.open {
            body.focus_trap("font-popup-trap", &focus)
                .into_any_element()
        } else {
            body.into_any_element()
        };
        root.child(motion::popup(body, "font-popup-motion", frame, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn unavailable_font_stays_selected_and_inheritance_is_a_separate_choice() {
        let families = vec!["Inter".into(), "Noto Sans".into()];
        let options = choices(&families, Some("Missing font"), "Из темы");
        assert_eq!(options.len(), 4);
        assert_eq!(options.iter().filter(|option| option.selected).count(), 1);
        assert_eq!(
            options.last().unwrap().value.as_deref(),
            Some("Missing font")
        );
        assert!(options.last().unwrap().selected);
        let inherited = choices(&families, None, "Как интерфейс");
        assert!(inherited[0].selected);
        assert_eq!(inherited[0].value, None);
        assert_eq!(choices(&families, Some("Inter"), "Из темы").len(), 3);
    }

    #[::core::prelude::v1::test]
    fn interface_and_chat_changes_and_resets_are_independent() {
        let mut prefs = Preferences::default();
        Target::Interface.apply(&mut prefs, Some("Interface font".into()));
        Target::Chat.apply(&mut prefs, Some("Chat font".into()));
        Target::Interface.apply(&mut prefs, None);
        assert_eq!(prefs.fonts.interface, None);
        assert_eq!(prefs.fonts.chat.as_deref(), Some("Chat font"));
        Target::Interface.apply(&mut prefs, Some("Another interface font".into()));
        Target::Chat.apply(&mut prefs, None);
        assert_eq!(
            prefs.fonts.interface.as_deref(),
            Some("Another interface font")
        );
        assert_eq!(prefs.fonts.chat, None);
    }
}
