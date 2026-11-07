// Адаптировано из Zed: crates/settings_ui/src/components/dropdown.rs (intrinsic dropdown width)
//! Building blocks of settings pages: sections, cards, rows, chips, radio
//! cards and select menus.

mod package_item;
pub(super) use package_item::{package_heading, package_icon};

use crate::ui::kit::ButtonLabel as _;
use crate::ui::kit::{Frame as _, RowRule as _};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::Pressable as _;
use crate::ui::prefs;

/// Inputs for numbers and other short values.
pub(super) const SHORT_INPUT_WIDTH: f32 = 112.;

pub(super) fn hashed_id(name: &'static str, key: &impl Hash) -> ElementId {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    ElementId::NamedInteger(name.into(), hasher.finish())
}

/// Marks unsaved edits in navigation, headings and rows.
pub(super) fn changed_dot(cx: &App) -> impl IntoElement + use<> {
    div()
        .size(px(6.))
        .flex_none()
        .rounded_full()
        .bg(cx.theme().warning)
}

/// Titled part of a page: heading with an optional hint and action, then
/// the body.
pub(super) fn section(
    title: impl Into<SharedString>,
    hint: Option<SharedString>,
    action: Option<AnyElement>,
    body: impl IntoElement,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    v_flex()
        .gap_2()
        .child(
            h_flex()
                .gap_3()
                .items_end()
                .px_1()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(div().text_sm().font_semibold().child(title.into()))
                        .children(hint.map(|hint| {
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(hint)
                        })),
                )
                .children(action),
        )
        .child(body)
}

/// Rounded card with quiet, inset hairline dividers between rows.
pub(super) fn card(rows: impl IntoIterator<Item = AnyElement>, cx: &App) -> AnyElement {
    kit::surface(
        group_box(cx).children(
            rows.into_iter()
                .enumerate()
                .map(|(ix, row)| card_cell(row, ix > 0, cx)),
        ),
        cx,
    )
}

/// The same divider and keyboard-reveal behavior in fixed and disclosed rows.
pub(super) fn card_cell(row: AnyElement, divider: bool, cx: &App) -> Div {
    super::reveal::cell(cx)
        .when(divider, |cell| cell.row_rule(cx))
        .child(row)
}

/// One surface for cards and every depth of foldable parameter groups.
fn group_box(cx: &App) -> Div {
    let theme = cx.theme();
    v_flex()
        .surface_frame(cx)
        .bg(theme.group_box)
        .overflow_hidden()
}

/// The whole disclosure owns its surface, including its persistent header.
/// The caller retains the common reveal animation around the body.
pub(super) fn nested_group(
    header: impl IntoElement,
    body: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    kit::surface(
        v_flex()
            .disclosure_frame(cx)
            .m_2()
            .child(header)
            .children(body),
        cx,
    )
}

/// Equal side insets, without a separate rule under the chevron.
pub(super) fn nested(body: impl IntoElement) -> Div {
    div().px_2().pb_2().child(body)
}

/// Muted explanatory text inside a card or under a heading.
pub(super) fn note(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .px_4()
        .py_3()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// Title on the left and a shrinking control on the right. Explicit multiline
/// controls go below; ordinary parameters stay on one line. The description lives in
/// a tooltip on the title, so rows stay one line high.
pub(super) fn row(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    control: impl IntoElement,
    vertical: bool,
    cx: &App,
) -> kit::SettingsRow {
    row_with_reset(title, description, control, vertical, None, cx)
}

/// A per-setting reset lives next to the caption, just like module overrides.
pub(super) fn row_with_reset(
    title: impl Into<SharedString>,
    description: Option<SharedString>,
    control: impl IntoElement,
    vertical: bool,
    reset: Option<AnyElement>,
    cx: &App,
) -> kit::SettingsRow {
    let title: SharedString = title.into();
    let id = hashed_id("row-title", &title);
    let title = h_flex()
        .min_w_0()
        .gap_1()
        .items_center()
        .child(described(id, title, description, None, cx))
        .children(reset);
    field_row(
        title,
        control,
        if vertical { Fit::Below } else { Fit::Fixed },
    )
}

/// How a row places its control.
#[derive(Clone, Copy)]
pub(super) enum Fit {
    /// At its content width on the right, shrinking with the available row.
    Fixed,
    /// A bounded single-line input on the right; it shrinks on narrow windows.
    Shrink,
    /// Under the title at full width.
    Below,
}

/// `row` with a title line built by the caller.
pub(super) fn field_row(
    title: impl IntoElement,
    control: impl IntoElement,
    fit: Fit,
) -> kit::SettingsRow {
    let control = match fit {
        Fit::Shrink => div()
            .w(prefs::scaled(220.))
            .min_w_0()
            .max_w_full()
            .child(control),
        _ => div().min_w_0().max_w_full().child(control),
    };
    kit::settings_row(title, control, matches!(fit, Fit::Below))
        .minimum_control_width(if matches!(fit, Fit::Shrink) { 64. } else { 36. })
}

/// One caption and optional information mark own one tooltip. Title clipping
/// and description preferences apply independently; config keys accompany
/// descriptions as semantic context rather than keyboard shortcuts.
pub(super) fn described(
    id: ElementId,
    title: SharedString,
    about: Option<SharedString>,
    key: Option<SharedString>,
    cx: &App,
) -> AnyElement {
    let about = about
        .map(|text| text.trim().to_owned().into())
        .filter(|text: &SharedString| !text.is_empty());
    let details = about.is_some() || key.is_some();
    crate::ui::fade::caption(id, title)
        .description(about)
        .context(key)
        .when(details, |caption| {
            caption.suffix(
                Icon::new(Lucide::Info)
                    .xsmall()
                    .flex_none()
                    .text_color(cx.theme().muted_foreground.opacity(0.7)),
            )
        })
        .into_any_element()
}

/// `described` for identifiers: tool, module and skill names.
pub(super) fn named(
    id: ElementId,
    name: impl Into<SharedString>,
    about: Option<SharedString>,
    key: Option<SharedString>,
    cx: &App,
) -> Div {
    div()
        .min_w_0()
        .font_family(cx.theme().mono_font_family.clone())
        .child(described(id, name.into(), about, key, cx))
}

/// Row of a listed item: its name on the left, chips and controls on the
/// right. Controls may wrap their own chips without moving the whole field.
pub(super) fn item_row(
    title: impl IntoElement,
    trailing: impl IntoIterator<Item = AnyElement>,
) -> kit::SettingsRow {
    field_row(
        title,
        h_flex()
            .flex_wrap()
            .gap_2()
            .items_center()
            .justify_end()
            .children(trailing),
        Fit::Fixed,
    )
}

/// Undo mark beside a title, for values set in the profile.
pub(super) fn reset_mark(
    id: ElementId,
    hint: &str,
    reset: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    kit::setting_reset(id, hint.to_owned(), reset, cx)
}

#[derive(Clone, Copy)]
pub(super) enum Tone {
    Muted,
    Accent,
    Success,
    Warning,
    Danger,
}

pub(super) fn chip(text: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    let theme = cx.theme();
    let color = match tone {
        Tone::Muted => theme.muted_foreground,
        Tone::Accent => theme.primary,
        Tone::Success => theme.success,
        Tone::Warning => theme.warning,
        Tone::Danger => theme.danger,
    };
    div()
        .flex_none()
        .px_1p5()
        .py(px(1.))
        .rounded(theme.radius / 2.)
        .text_xs()
        .text_color(color)
        .bg(color.opacity(0.1))
        .child(text.into())
}

pub(super) fn mono(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .font_family(cx.theme().mono_font_family.clone())
        .child(text.into())
}

/// Monospace label on one line that fades out where it runs out of room.
pub(super) fn mono_line(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .min_w_0()
        .font_family(cx.theme().mono_font_family.clone())
        .child(faded(text))
}

/// Selectable card of a radio group.
pub(super) fn radio_card(
    id: ElementId,
    selected: bool,
    title: impl IntoElement,
    hint: Option<SharedString>,
    chips: Vec<Div>,
    select: impl Fn(&mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let dot = div()
        .mt(px(2.))
        .size(prefs::scaled(16.))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(if selected {
            theme.primary
        } else {
            theme.border
        })
        .flex()
        .items_center()
        .justify_center()
        .when(selected, |dot| {
            dot.child(div().size(px(8.)).rounded_full().bg(theme.primary))
        });
    h_flex()
        .id(id)
        .gap_3()
        .items_start()
        .px_4()
        .py_3()
        .surface_frame(cx)
        .cursor_pointer()
        .pressable(cx)
        .map(|card| {
            if selected {
                card.bg(kit::surface_selected(cx))
                    .text_color(theme.group_box_foreground)
            } else {
                card.bg(theme.group_box)
                    .hover(|card| card.bg(kit::surface_hover(cx)))
            }
        })
        .child(dot)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(div().text_sm().font_medium().child(title))
                .children(hint.map(|hint| {
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(hint)
                }))
                .when(!chips.is_empty(), |text| {
                    text.child(h_flex().flex_wrap().gap_1().children(chips))
                }),
        )
        .on_click(move |_, _, cx| select(cx))
}

/// Row that opens or closes a nested block.
pub(super) fn disclosure(
    id: ElementId,
    open: bool,
    label: impl IntoElement,
    changed: bool,
    toggle: impl Fn(&mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let chevron_id = ElementId::Name(format!("{id}-chevron").into());
    h_flex()
        .id(id)
        .px_4()
        .py_2p5()
        .gap_2()
        .items_center()
        .text_sm()
        .disclosure_surface(cx)
        .child(crate::ui::motion::chevron(
            chevron_id,
            open,
            theme.muted_foreground,
        ))
        .child(label)
        .when(changed && !open, |row| row.child(changed_dot(cx)))
        .on_click(move |_, _, cx| toggle(cx))
}

pub(super) struct MenuOption<T> {
    pub label: SharedString,
    pub value: T,
    pub checked: bool,
}

/// Filled button opening a single-choice list; `select` receives its value.
pub(super) fn select_menu<T: Clone + 'static>(
    id: ElementId,
    label: impl Into<SharedString>,
    options: Vec<MenuOption<T>>,
    select: impl Fn(T, &mut App) + 'static,
) -> AnyElement {
    let select = Rc::new(select);
    let options = Rc::new(options);
    let id: ElementId = id.into();
    crate::ui::context::menu_button(
        ElementId::Name(format!("settings-menu-{id:?}").into()),
        kit::button(id)
            .faded_label(label)
            .px_1p5()
            .small()
            .min_w_0()
            .max_w_full(),
        true,
        move |menu: PopupMenu, _, _| {
            // Intrinsic width and viewport bounds come from the common menu.
            options.iter().fold(menu, |menu, option| {
                let (select, value) = (select.clone(), option.value.clone());
                menu.item(
                    crate::ui::selection::menu_option(option.label.clone(), option.checked)
                        .on_click(move |_, _, cx| select(value.clone(), cx)),
                )
            })
        },
    )
    .settings_selector()
    .min_w_0()
    .max_w_full()
    .into_any_element()
}

/// Small muted line, skipped when the text is blank.
pub(super) fn hint_text(text: impl Into<String>, cx: &App) -> Option<Div> {
    let text = text.into();
    (!text.trim().is_empty()).then(|| {
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(text)
    })
}

// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (SubPageLink).
// The same text action opens a settings destination or its local instructions.
pub(super) fn settings_action(
    id: ElementId,
    label: impl Into<SharedString>,
    cx: &App,
) -> gpui_kit::component::button::Button {
    kit::button(id).faded_label(label).quiet(cx).xsmall().child(
        Icon::new(Lucide::ChevronRight)
            .xsmall()
            .text_color(cx.theme().muted_foreground),
    )
}

// Every internal link follows the same exact destination.
impl super::SettingsView {
    pub(super) fn settings_link(
        &self,
        id: ElementId,
        label: impl Into<SharedString>,
        target: super::spot::Destination,
        cx: &Context<Self>,
    ) -> AnyElement {
        settings_action(id, label, cx)
            .on_click(cx.listener(move |this, _, window, cx| this.go_to(&target, window, cx)))
            .into_any_element()
    }
}
