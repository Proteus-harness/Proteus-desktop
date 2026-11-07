// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (render_settings_item_layout)
//! Client pages: preferences of this device, applied immediately.

use gpui_kit::component::*;
use gpui_kit::*;

use super::SettingsView;
use super::sizes::slider;
use super::widgets::{
    MenuOption, card, hashed_id, reset_mark, row_with_reset, section, select_menu,
};
use crate::preferences::{
    LabelTruncation, MotionPreference, Notifications, Preference, Preferences, SendMode,
    ThemePreference, TitleHint,
};
use crate::ui::kit;
use crate::ui::prefs::{self, ClientPreferences};
use crate::ui::theme::{self, ThemeProblems};

const THEMES: [(ThemePreference, &str); 3] = [
    (ThemePreference::System, "Как в системе"),
    (ThemePreference::Dark, "Тёмная"),
    (ThemePreference::Light, "Светлая"),
];

const MOTIONS: [(MotionPreference, &str); 3] = [
    (MotionPreference::Full, "Полные"),
    (MotionPreference::Reduced, "Уменьшенные"),
    (MotionPreference::Off, "Выключены"),
];

const TRUNCATIONS: [(LabelTruncation, &str); 2] = [
    (LabelTruncation::Fade, "Плавное затемнение"),
    (LabelTruncation::Ellipsis, "Многоточие"),
];

const HINT_TITLES: [(TitleHint, &str); 3] = [
    (TitleHint::Clipped, "Только обрезанное"),
    (TitleHint::Always, "Всегда"),
    (TitleHint::Never, "Никогда"),
];

const SCALES: [(u16, &str); 9] = [
    (75, "75%"),
    (80, "80%"),
    (90, "90%"),
    (100, "100%"),
    (110, "110%"),
    (125, "125%"),
    (150, "150%"),
    (175, "175%"),
    (200, "200%"),
];

const SEND_MODES: [(SendMode, &str); 2] = [
    (SendMode::Enter, "Enter"),
    (SendMode::CtrlEnter, "Ctrl + Enter"),
];

const NOTIFICATIONS: [(Notifications, &str); 2] = [
    (Notifications::Background, "Когда окно в фоне"),
    (Notifications::Off, "Выключены"),
];

/// Title and explanation of a client row, shared with the settings search.
pub(super) struct RowText {
    pub(super) field: Preference,
    pub(super) title: &'static str,
    pub(super) about: Option<&'static str>,
}

const fn text(field: Preference, title: &'static str, about: Option<&'static str>) -> RowText {
    RowText {
        field,
        title,
        about,
    }
}

const THEME: RowText = text(Preference::Theme, "Тема", None);
const DARK_PALETTE: RowText = text(
    Preference::DarkPalette,
    "Тёмная палитра",
    Some("Готовые палитры Catppuccin, Tokyo Night, Gruvbox, Everforest, Solarized и Ayu."),
);
const LIGHT_PALETTE: RowText = text(
    Preference::LightPalette,
    "Светлая палитра",
    Some(
        "Свои темы в формате gpui-kit кладите в ~/.config/dev.proteus.native/themes/; они читаются при запуске.",
    ),
);
const SCALE: RowText = text(
    Preference::Scale,
    "Масштаб",
    Some("Ctrl + = и Ctrl + − меняют масштаб, Ctrl + 0 возвращает 100%."),
);
const SURFACE_RADIUS: RowText = text(
    Preference::SurfaceRadius,
    "Скругление углов",
    Some(
        "Общий радиус поля сообщения, раскрытых виджетов, меню и подсветки. Кнопки и компактные поля используют половину радиуса. Размер указан для масштаба 100%.",
    ),
);
const MOTION: RowText = text(
    Preference::Motion,
    "Анимации",
    Some("Уменьшенные оставляют плавное появление без сдвигов; выключенные показывают всё сразу."),
);
const HINT_TITLE: RowText = text(
    Preference::HintTitle,
    "Название в подсказке",
    Some("Показывать полное название при наведении: только если оно обрезано, всегда или никогда."),
);
const HINT_DESCRIPTION: RowText = text(
    Preference::HintDescription,
    "Описание в подсказке",
    Some("Пояснение действия или параметра при наведении."),
);
const HINT_SHORTCUT: RowText = text(
    Preference::HintShortcut,
    "Клавиши в подсказке",
    Some("Сочетание клавиш, если оно назначено этому действию."),
);
const TRUNCATION: RowText = text(
    Preference::LabelTruncation,
    "Длинные подписи",
    Some("Как обрезать названия шрифтов и другие подписи, которые не помещаются в строку."),
);
const SEND: RowText = text(
    Preference::Send,
    "Отправка сообщения",
    Some(
        "Shift+Enter всегда переносит строку. Во время ответа отправка добавляет сообщение в очередь.",
    ),
);
const TEXT_SIZE: RowText = text(
    Preference::ChatText,
    "Размер текста",
    Some("Сообщения диалога и поле ввода."),
);
const INTERFACE_FONT: RowText = text(
    Preference::InterfaceFont,
    "Шрифт интерфейса",
    Some(
        "Установленный шрифт для панелей, меню и настроек. Очистите выбор, чтобы использовать шрифт темы.",
    ),
);
const CHAT_FONT: RowText = text(
    Preference::ChatFont,
    "Шрифт чата",
    Some(
        "Сообщения и поле ввода. Очистите выбор, чтобы наследовать шрифт интерфейса. Код сохраняет моноширинный шрифт.",
    ),
);
const CHAT_WIDTH: RowText = text(
    Preference::ChatWidth,
    "Ширина диалога",
    Some("Наибольшая ширина ленты и поля ввода."),
);
const COMPACT_TOOLS: RowText = text(
    Preference::CompactTools,
    "Компактные цепочки инструментов",
    Some("Три и больше вызова подряд сворачиваются в одну строку."),
);
const AUTOSCROLL: RowText = text(
    Preference::Autoscroll,
    "Автопрокрутка",
    Some("Плавно следовать за ответом, пока лента внизу."),
);
const NOTIFICATIONS_ROW: RowText = text(
    Preference::Notifications,
    "Уведомления",
    Some("Когда окно в фоне: агент закончил, ждёт подтверждения или ответа."),
);

pub(super) const APPEARANCE_ROWS: [&RowText; 11] = [
    &THEME,
    &DARK_PALETTE,
    &LIGHT_PALETTE,
    &INTERFACE_FONT,
    &SCALE,
    &SURFACE_RADIUS,
    &MOTION,
    &TRUNCATION,
    &HINT_TITLE,
    &HINT_DESCRIPTION,
    &HINT_SHORTCUT,
];
pub(super) const CHAT_ROWS: [&RowText; 7] = [
    &SEND,
    &CHAT_FONT,
    &TEXT_SIZE,
    &CHAT_WIDTH,
    &COMPACT_TOOLS,
    &AUTOSCROLL,
    &NOTIFICATIONS_ROW,
];

impl SettingsView {
    fn reset_preference(&mut self, field: Preference, window: &mut Window, cx: &mut Context<Self>) {
        prefs::update(cx, |prefs| field.reset(prefs));
        match field {
            Preference::InterfaceFont => self.fonts.reset_interface(window, cx),
            Preference::ChatFont => self.fonts.reset_chat(window, cx),
            _ => self.sizes.reset(field, window, cx),
        }
        cx.notify();
    }

    fn text_row(
        &self,
        text: &RowText,
        control: impl IntoElement,
        cx: &Context<Self>,
    ) -> AnyElement {
        let field = text.field;
        let reset = (!field.is_default(prefs::current(cx))).then(|| {
            let view = cx.entity().downgrade();
            let hint = match field {
                Preference::InterfaceFont => "Вернуть шрифт темы",
                Preference::ChatFont => "Наследовать шрифт интерфейса",
                _ => "Вернуть значение по умолчанию",
            };
            reset_mark(
                hashed_id("reset-preference", &text.title),
                hint,
                move |window, cx| {
                    let _ = view.update(cx, |view, cx| view.reset_preference(field, window, cx));
                },
                cx,
            )
        });
        self.spotted(
            &super::spot::row(text.title),
            row_with_reset(
                text.title,
                text.about.map(Into::into),
                control,
                false,
                reset,
                cx,
            )
            .minimum_control_width(match field {
                Preference::SurfaceRadius | Preference::ChatText | Preference::ChatWidth => 120.,
                Preference::InterfaceFont | Preference::ChatFont => 64.,
                _ => 36.,
            }),
            cx,
        )
    }
    pub(super) fn appearance_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let current = prefs::current(cx);
        let themes = [
            self.text_row(
                &THEME,
                choice("pref-theme", &THEMES, current.theme, |prefs, value| {
                    prefs.theme = value
                }),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &DARK_PALETTE,
                theme_choice(
                    "pref-dark-theme",
                    true,
                    &current.themes.dark,
                    cx,
                    |prefs, name| prefs.themes.dark = name,
                ),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &LIGHT_PALETTE,
                theme_choice(
                    "pref-light-theme",
                    false,
                    &current.themes.light,
                    cx,
                    |prefs, name| prefs.themes.light = name,
                ),
                cx,
            )
            .into_any_element(),
        ];
        let typography = [
            self.text_row(&INTERFACE_FONT, self.fonts.interface(), cx)
                .into_any_element(),
            self.text_row(
                &SCALE,
                choice("pref-scale", &SCALES, current.view.scale, |prefs, value| {
                    prefs.view.scale = value
                }),
                cx,
            )
            .into_any_element(),
        ];
        let interface = [
            self.text_row(
                &SURFACE_RADIUS,
                slider(&self.sizes.surface_radius, "px", cx),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &MOTION,
                choice("pref-motion", &MOTIONS, current.motion, |prefs, value| {
                    prefs.motion = value
                }),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &TRUNCATION,
                choice(
                    "pref-truncation",
                    &TRUNCATIONS,
                    current.label_truncation,
                    |prefs, value| prefs.label_truncation = value,
                ),
                cx,
            )
            .into_any_element(),
        ];
        let hints = [
            self.text_row(
                &HINT_TITLE,
                choice(
                    "pref-hint-title",
                    &HINT_TITLES,
                    current.hints.title,
                    |prefs, value| prefs.hints.title = value,
                ),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &HINT_DESCRIPTION,
                kit::switch("pref-hint-description")
                    .checked(current.hints.description)
                    .on_click(|checked, _, cx| {
                        let checked = *checked;
                        prefs::update(cx, move |prefs| prefs.hints.description = checked)
                    }),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &HINT_SHORTCUT,
                kit::switch("pref-hint-shortcut")
                    .checked(current.hints.shortcut)
                    .on_click(|checked, _, cx| {
                        let checked = *checked;
                        prefs::update(cx, move |prefs| prefs.hints.shortcut = checked)
                    }),
                cx,
            )
            .into_any_element(),
        ];
        let mut parts = self.preferences_problem(cx);
        parts.extend(cx.global::<ThemeProblems>().0.iter().map(|problem| {
            div()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(format!("Тема не прочитана: {problem}"))
                .into_any_element()
        }));
        parts.extend([
            section("Тема", None, None, card(themes, cx), cx).into_any_element(),
            section("Шрифт и масштаб", None, None, card(typography, cx), cx).into_any_element(),
            section("Интерфейс", None, None, card(interface, cx), cx).into_any_element(),
            section("Подсказки", None, None, card(hints, cx), cx).into_any_element(),
        ]);
        parts
    }

    pub(super) fn chat_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let current = prefs::current(cx);
        let rows = [self
            .text_row(
                &SEND,
                choice("pref-send", &SEND_MODES, current.send, |prefs, value| {
                    prefs.send = value
                }),
                cx,
            )
            .into_any_element()];
        let sizes = [
            self.text_row(&CHAT_FONT, self.fonts.chat(), cx)
                .into_any_element(),
            self.text_row(&TEXT_SIZE, slider(&self.sizes.chat_text, "px", cx), cx)
                .into_any_element(),
            self.text_row(&CHAT_WIDTH, slider(&self.sizes.chat_width, "px", cx), cx)
                .into_any_element(),
            self.text_row(
                &COMPACT_TOOLS,
                kit::switch("pref-compact-tools")
                    .checked(current.view.compact_tools)
                    .on_click(|checked, _, cx| {
                        let checked = *checked;
                        prefs::update(cx, move |prefs| prefs.view.compact_tools = checked)
                    })
                    .into_any_element(),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &AUTOSCROLL,
                kit::switch("pref-autoscroll")
                    .checked(current.view.autoscroll)
                    .on_click(|checked, _, cx| {
                        let checked = *checked;
                        prefs::update(cx, move |prefs| prefs.view.autoscroll = checked)
                    })
                    .into_any_element(),
                cx,
            )
            .into_any_element(),
            self.text_row(
                &NOTIFICATIONS_ROW,
                choice(
                    "pref-notifications",
                    &NOTIFICATIONS,
                    current.notifications,
                    |prefs, value| prefs.notifications = value,
                ),
                cx,
            )
            .into_any_element(),
        ];
        let mut parts = self.preferences_problem(cx);
        parts.push(section("Диалог", None, None, card(sizes, cx), cx).into_any_element());
        parts.push(section("Поле ввода", None, None, card(rows, cx), cx).into_any_element());
        parts
    }

    pub(super) fn preferences_problem(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        cx.global::<ClientPreferences>()
            .problem
            .clone()
            .map(|problem| {
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(problem)
                    .into_any_element()
            })
            .into_iter()
            .collect()
    }
}

fn theme_choice(
    id: &'static str,
    dark: bool,
    current: &str,
    cx: &App,
    apply: fn(&mut Preferences, String),
) -> AnyElement {
    let items = theme::names(dark, cx)
        .into_iter()
        .map(|name| MenuOption {
            label: theme::label(&name),
            checked: name.as_ref() == current,
            value: name,
        })
        .collect();
    select_menu(id.into(), theme::label(current), items, move |name, cx| {
        prefs::update(cx, |prefs| apply(prefs, name.to_string()))
    })
}

fn choice<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: T,
    apply: fn(&mut Preferences, T),
) -> AnyElement {
    let label = options
        .iter()
        .find(|(value, _)| *value == current)
        .map(|(_, label)| *label)
        .unwrap_or_default();
    let items = options
        .iter()
        .map(|(value, label)| MenuOption {
            label: (*label).into(),
            value: *value,
            checked: *value == current,
        })
        .collect();
    select_menu(id.into(), label, items, move |value, cx| {
        prefs::update(cx, |prefs| apply(prefs, value))
    })
}
