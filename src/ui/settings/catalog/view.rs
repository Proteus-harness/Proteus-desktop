// Адаптировано из Zed: crates/extensions_ui/src/components/extension_card.rs (fixed card geometry and trailing actions).
//! Catalog page: header, toolbar, the entry list and the details beside it.

use crate::ui::kit::ButtonLabel as _;
use crate::ui::kit::Frame as _;
use std::ops::Range;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::super::{
    SettingsView,
    widgets::{Tone, chip, hashed_id, package_heading, package_icon},
};
use super::details::DETAILS_WIDTH;
use super::items::{Item, Status, kind_icon, version_label};
use crate::catalog::open::OPEN;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::Hint as _;
use crate::ui::prefs;
use crate::ui::{Spinner, motion};

impl SettingsView {
    pub(in crate::ui::settings) fn catalog_page(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let opened = self.catalogs.selected.is_some();
        let phase = motion::blend(
            "catalog-details-motion".into(),
            opened,
            motion::BASE,
            window,
            cx,
        );
        if !opened
            && phase == 0.
            && let Some(closing) = self.catalogs.closing.clone()
        {
            let owner = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |this, cx| {
                    if this.catalogs.selected.is_none()
                        && this.catalogs.closing.as_ref() == Some(&closing)
                    {
                        this.catalogs.closing = None;
                        this.catalogs.readme = None;
                        this.catalogs.readme_changes = None;
                        cx.notify();
                    }
                });
            });
        }
        let theme = cx.theme();
        let sources = &prefs::current(cx).catalogs.sources;
        let installed = &prefs::current(cx).catalogs.installed;
        let items = self.catalog_items(installed);
        let updates = items
            .iter()
            .filter(|item| item.status == Status::Update)
            .count();
        let shown = items
            .iter()
            .filter(|item| self.catalog_item_shown(item))
            .collect::<Vec<_>>();
        let selected = self
            .catalogs
            .selected
            .as_ref()
            .or(self.catalogs.closing.as_ref())
            .filter(|_| phase > 0.)
            .and_then(|(source, id)| {
                items
                    .iter()
                    .find(|item| item.source == source && item.entry.id == *id)
            });
        let loading = &self.catalogs.loading;
        let width = self.catalogs.width.clone();
        // Until the first layout the page is taken for wide.
        let measured = if width.get() > 0. { width.get() } else { 1200. };
        let full = measured < 960.;
        let extent = if motion::moves() {
            phase
        } else if phase > 0. {
            1.
        } else {
            0.
        };
        let list_width = if selected.is_some() && !full {
            measured - DETAILS_WIDTH * extent
        } else {
            measured
        };
        let body = if sources.is_empty() {
            self.catalog_empty(
                "Источников пока нет",
                "Подключите открытые каталоги skills или свой git-репозиторий.",
                true,
                cx,
            )
        } else if items.is_empty() && !loading.is_empty() {
            self.catalog_empty(
                "Скачиваю каталоги…",
                "Записи появятся, как только скачается первый источник.",
                false,
                cx,
            )
        } else if !self.catalog_browsing() && !items.is_empty() {
            self.catalog_overview(&items, selected.is_some(), list_width, cx)
        } else if shown.is_empty() {
            self.catalog_empty(
                if items.is_empty() {
                    "Каталоги пусты"
                } else {
                    "Ничего не найдено"
                },
                if items.is_empty() {
                    "Обновите источники или проверьте их в окне «Источники»."
                } else {
                    "Измените поиск или фильтры."
                },
                false,
                cx,
            )
        } else {
            // Only the rows in sight are built; the arrows scroll by index.
            let narrow = selected.is_some();
            div()
                .id("catalog-list")
                .relative()
                .pt_3()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    uniform_list(
                        "catalog-rows",
                        shown.len(),
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            let installed = &prefs::current(cx).catalogs.installed;
                            let profile = this.profile_blocked();
                            this.catalog_items(installed)
                                .iter()
                                .filter(|item| this.catalog_item_shown(item))
                                .skip(range.start)
                                .take(range.len())
                                .map(|item| this.catalog_row(item, narrow, profile, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full()
                    .track_scroll(&self.catalogs.scroll),
                )
                .vertical_scrollbar(&self.catalogs.scroll)
                .into_any_element()
        };
        let mode_key = format!(
            "{:?}-{:?}-{}-{}-{}-{}",
            self.catalogs.show,
            self.catalogs.kind,
            self.catalogs
                .scope
                .as_ref()
                .map(|scope| scope.source.key())
                .unwrap_or_default(),
            self.catalogs
                .scope
                .as_ref()
                .and_then(|scope| scope.category.as_deref())
                .unwrap_or_default(),
            self.catalogs.query.is_empty(),
            items.is_empty()
        );
        let body = motion::switch_in(
            div().size_full().min_w_0().child(body),
            hashed_id("catalog-mode", &mode_key),
        );
        let page = v_flex()
            .size_full()
            .child(self.catalog_header(sources.len(), cx))
            .child(self.catalog_toolbar(updates, cx))
            .children(self.catalogs.message.as_ref().map(|(error, text)| {
                h_flex()
                    .px_6()
                    .pb_3()
                    .gap_2()
                    .items_start()
                    .text_sm()
                    .text_color(if *error {
                        theme.danger
                    } else if self.catalogs.busy {
                        theme.muted_foreground
                    } else {
                        theme.success
                    })
                    .child(if self.catalogs.busy && !*error {
                        Spinner::new().small().into_any_element()
                    } else {
                        Icon::new(if *error {
                            IconName::TriangleAlert
                        } else {
                            IconName::Check
                        })
                        .small()
                        .flex_none()
                        .mt_0p5()
                        .into_any_element()
                    })
                    .child(div().flex_1().min_w_0().child(text.clone()))
            }))
            .when(!loading.is_empty(), |page| {
                let names = loading
                    .iter()
                    .map(|source| source.name())
                    .collect::<Vec<_>>();
                page.child(
                    h_flex()
                        .px_6()
                        .pb_3()
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(Spinner::new().small())
                        .child(faded(format!("Скачиваю: {}", names.join(", ")))),
                )
            })
            .child(
                self.catalog_list_keys(h_flex())
                    .flex_1()
                    .min_h_0()
                    .border_t_1()
                    .border_color(theme.transparent)
                    .relative()
                    .overflow_hidden()
                    .child(div().flex_1().min_w_0().h_full().child(body))
                    .children(selected.map(|item| {
                        let width = if full { measured } else { DETAILS_WIDTH };
                        let content = motion::switch_in(
                            div()
                                .w(prefs::scaled(width))
                                .flex_none()
                                .h_full()
                                .child(self.catalog_details(item, full, cx)),
                            hashed_id("catalog-detail-content", &item.key()),
                        );
                        h_flex()
                            .id("catalog-details-slot")
                            .flex_none()
                            .h_full()
                            .overflow_hidden()
                            .w(prefs::scaled(width * extent))
                            .opacity(phase)
                            .bg(theme.background)
                            .when(full, |pane| pane.absolute().top_0().right_0().occlude())
                            .when(!opened, |pane| {
                                pane.capture_any_mouse_down(|_, window, cx| {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                })
                                .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
                            })
                            .child(content)
                    }))
                    .on_prepaint(move |bounds, window, _| {
                        let next = prefs::unscaled(bounds.size.width, window);
                        if (width.get() - next).abs() > 1. {
                            width.set(next);
                            window.refresh();
                        }
                    }),
            );
        self.catalog_page_keys(page, cx).into_any_element()
    }

    pub(super) fn catalog_row(
        &self,
        item: &Item,
        _narrow: bool,
        profile: Option<&'static str>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selected = self
            .catalogs
            .selected
            .as_ref()
            .is_some_and(|(source, id)| item.key() == (source, id.as_str()));
        let (source, entry) = (item.source.clone(), item.entry.clone());
        let mut facts = vec![version_label(&entry.version)];
        if self.catalogs.kind.is_none() {
            facts.push(item.entry.kind.title().to_string());
        }
        for fact in item
            .category()
            .filter(|_| {
                !self
                    .catalogs
                    .scope
                    .as_ref()
                    .is_some_and(|scope| scope.category.is_some())
            })
            .into_iter()
            .chain(
                (!_narrow && self.catalogs.scope.is_none()).then_some(item.catalog.title.as_str()),
            )
        {
            if !facts.iter().any(|known| known == fact) {
                facts.push(fact.to_owned());
            }
        }
        // UniformList measures one row for every entry. Keep the tag strip
        // clipped to one line even when the details pane narrows the list.
        let tags = h_flex()
            .w_full()
            .min_w_0()
            .h(prefs::scaled(20.))
            .gap_1()
            .items_center()
            .overflow_hidden()
            .whitespace_nowrap()
            .children(facts.into_iter().map(|fact| chip(fact, Tone::Muted, cx)));
        let heading = package_heading(
            kind_icon(entry.kind).text_color(theme.muted_foreground),
            div()
                .min_w_0()
                .h(prefs::scaled(20.))
                .child(faded(entry.title.clone())),
            [tags.into_any_element()],
        );
        let card = h_flex()
            .id(hashed_id("catalog-row", &item.key()))
            .w_full()
            .min_w_0()
            .px_4()
            .py_2p5()
            .gap_3()
            .items_center()
            .surface_frame(cx)
            .cursor_pointer()
            .map(|row| {
                if selected {
                    row.bg(kit::surface_selected(cx))
                } else {
                    row.bg(theme.group_box)
                        .hover(|row| row.bg(kit::surface_hover(cx)))
                }
            })
            .child(heading)
            .child(
                div()
                    .id(hashed_id("catalog-row-action", &item.key()))
                    .flex_none()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(self.catalog_action(item, "row", profile, cx)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                if this
                    .catalogs
                    .selected
                    .as_ref()
                    .is_some_and(|(old, id)| *old == source && *id == entry.id)
                {
                    this.deselect_catalog_entry(cx);
                } else {
                    this.select_catalog_entry(source.clone(), entry.clone(), cx);
                }
            }));
        // The inset is outside the card, so its whole visible surface remains
        // clickable and neighbouring entries have a real eight-pixel gap.
        div()
            .w_full()
            .min_w_0()
            .px_6()
            .pb_2()
            .child(card)
            .into_any_element()
    }

    /// The main action of an entry: install, connect again or update; an
    /// installed entry gets a mark instead.
    pub(super) fn catalog_action(
        &self,
        item: &Item,
        place: &'static str,
        profile: Option<&'static str>,
        cx: &Context<Self>,
    ) -> AnyElement {
        if item
            .entry
            .source
            .as_ref()
            .is_some_and(|source| source.artifact.is_none())
        {
            let (source, entry) = (item.source.clone(), item.entry.clone());
            let working = self.catalogs.busy
                && self
                    .catalogs
                    .working
                    .as_ref()
                    .is_some_and(|(source, id)| item.key() == (source, id.as_str()));
            return kit::button(hashed_id("catalog-source-download", &(place, item.key())))
                .faded_label("Скачать исходники")
                .small()
                .loading(working)
                .loading_icon(IconName::LoaderCircle)
                .disabled(!self.catalog_idle() || self.catalogs.loading.contains(item.source))
                .hint("Полный workspace и инструкция ручной сборки", cx)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.download_catalog_source(source.clone(), entry.clone(), cx)
                }))
                .into_any_element();
        }
        let label = match item.status {
            Status::Available => "Установить",
            Status::Detached => "Подключить",
            Status::Update => "Обновить",
            Status::Broken => "Переустановить",
            Status::Installed => {
                let theme = cx.theme();
                return h_flex()
                    .w(prefs::scaled(120.))
                    .map(|row| {
                        if place == "details" {
                            row.justify_start()
                        } else {
                            row.justify_end()
                        }
                    })
                    .gap_1()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        Icon::new(Lucide::CircleCheck)
                            .xsmall()
                            .text_color(theme.success),
                    )
                    .child("Установлено")
                    .into_any_element();
            }
        };
        let blocked = self.catalog_install_gate(item.source, item.entry.kind, profile);
        let working = self.catalogs.busy
            && self
                .catalogs
                .working
                .as_ref()
                .is_some_and(|(source, id)| item.key() == (source, id.as_str()));
        let (source, entry) = (item.source.clone(), item.entry.clone());
        kit::button(hashed_id("catalog-install", &(place, item.key())))
            .w(prefs::scaled(120.))
            .faded_label(label)
            .small()
            .map(|button| {
                if matches!(
                    item.status,
                    Status::Available | Status::Update | Status::Detached
                ) {
                    button.primary()
                } else {
                    button
                }
            })
            .loading(working)
            .loading_icon(IconName::LoaderCircle)
            .disabled(!self.catalog_idle() || blocked.is_some())
            .when_some(blocked, |button, reason| button.hint(reason, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.install_catalog_entry(source.clone(), entry.clone(), cx)
            }))
            .into_any_element()
    }

    fn catalog_empty(
        &self,
        title: &'static str,
        text: &'static str,
        add: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .items_center()
            .justify_center()
            .gap_2()
            .p_8()
            .child(
                Icon::new(Lucide::Library)
                    .large()
                    .text_color(theme.muted_foreground),
            )
            .child(div().font_medium().child(title))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(text),
            )
            .when(add, |empty| {
                empty.child(
                    h_flex()
                        .gap_2()
                        .mt_2()
                        .child(
                            kit::button("catalog-add-open")
                                .faded_label("Добавить открытые каталоги")
                                .primary()
                                .small()
                                .loading(!self.catalogs.loading.is_empty())
                                .loading_icon(IconName::LoaderCircle)
                                .disabled(!self.catalogs.loading.is_empty())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let sources = OPEN.iter().map(|open| open.source()).collect();
                                    this.add_catalog_sources(sources, cx)
                                })),
                        )
                        .child(
                            kit::button("catalog-add-first")
                                .faded_label("Источники")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_catalog_sources(window, cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

/// The same icon column as package rows, on the inspector's quiet badge.
pub(super) fn kind_badge(item: &Item, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    package_icon(kind_icon(item.entry.kind).text_color(theme.muted_foreground))
        .rounded(theme.radius)
        .bg(theme.muted)
}
