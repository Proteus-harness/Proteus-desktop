//! Details of the selected catalog entry beside the list.

use crate::ui::kit::ButtonLabel as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::super::{
    SettingsView,
    widgets::{self, Tone, chip, hashed_id},
};
use super::items::{Item, Status, version_label};
use super::view::kind_badge;
use crate::catalog::schema::Kind;
use crate::ui::kit;
use crate::ui::kit::Hint as _;
use crate::ui::prefs;

pub(super) const DETAILS_WIDTH: f32 = 380.;

impl SettingsView {
    /// `full` gives the pane the whole width when the list beside it
    /// would get too narrow.
    pub(super) fn catalog_details(
        &self,
        item: &Item,
        full: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let entry = item.entry;
        let mut facts = vec![
            chip(entry.kind.title(), Tone::Muted, cx),
            chip(version_label(&entry.version), Tone::Muted, cx),
        ];
        if let Some(category) = item.category() {
            facts.push(chip(category.to_string(), Tone::Muted, cx));
        }
        match (item.status, item.installed) {
            (Status::Update, Some(old)) => facts.push(chip(
                format!("установлена {}", version_label(&old.version)),
                Tone::Warning,
                cx,
            )),
            (Status::Detached, _) => facts.push(chip("не подключено", Tone::Warning, cx)),
            (Status::Broken, _) => facts.push(chip("ошибка пакета", Tone::Warning, cx)),
            _ => {}
        }
        let notes = [
            if entry
                .source
                .as_ref()
                .is_some_and(|source| source.artifact.is_some())
            {
                Some(match entry.kind {
                    Kind::ProcessModule => {
                        "После установки выберите инструменты или реализацию слота в профиле."
                    }
                    Kind::ClientExtension => "Расширение применяется сразу после установки.",
                    _ => "Пакет собирается локально.",
                })
            } else if entry.source.is_some() {
                Some(
                    "Скачивается полный workspace. Сборка выполняется вручную; команда и требования указаны ниже и в BUILD.md рядом с исходниками.",
                )
            } else {
                match entry.kind {
                    Kind::McpServer => Some("Сервер записывается в профиль сразу при установке."),
                    Kind::AgentPlugin => Some(
                        "Пакет подключается к профилю сразу при установке. Его файлы остаются на диске после удаления.",
                    ),
                    Kind::ClientExtension | Kind::Skill | Kind::ProcessModule => None,
                }
            },
            self.catalog_install_blocked(item.source, entry.kind),
        ];
        let removable = item
            .installed
            .filter(|_| item.status != Status::Available && entry.kind != Kind::ProcessModule)
            .cloned();
        div()
            .id(hashed_id("catalog-details", &item.key()))
            .map(|pane| {
                if full {
                    pane.flex_1().min_w_0()
                } else {
                    pane.w(prefs::scaled(DETAILS_WIDTH))
                        .flex_none()
                        .border_l_1()
                        .border_color(theme.border)
                }
            })
            .h_full()
            .child(
                v_flex()
                    .px_6()
                    .py_5()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_3()
                            .items_start()
                            .child(kind_badge(item, cx))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div().text_lg().font_semibold().child(entry.title.clone()),
                                    )
                                    .when(entry.title != entry.id, |header| {
                                        header.child(
                                            div()
                                                .text_xs()
                                                .text_color(theme.muted_foreground)
                                                .font_family(theme.mono_font_family.clone())
                                                .child(entry.id.clone()),
                                        )
                                    }),
                            )
                            .child(
                                kit::button("catalog-details-close")
                                    .icon(IconName::Close)
                                    .quiet(cx)
                                    .xsmall()
                                    .hint("Закрыть", cx)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.deselect_catalog_entry(cx)
                                    })),
                            ),
                    )
                    .child(h_flex().flex_wrap().gap_1p5().children(facts))
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(self.catalog_action(item, "details", self.profile_blocked(), cx))
                            .children(
                                (item.status == Status::Installed
                                    && entry
                                        .source
                                        .as_ref()
                                        .is_some_and(|source| source.artifact.is_some()))
                                .then(|| {
                                    let source = item.source.clone();
                                    let entry = entry.clone();
                                    kit::button("catalog-reinstall")
                                        .faded_label("Переустановить")
                                        .quiet(cx)
                                        .small()
                                        .disabled(
                                            !self.catalog_idle()
                                                || self
                                                    .catalog_install_blocked(
                                                        item.source,
                                                        entry.kind,
                                                    )
                                                    .is_some(),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.install_catalog_entry(
                                                source.clone(),
                                                entry.clone(),
                                                cx,
                                            )
                                        }))
                                }),
                            )
                            .children(removable.map(|installed| {
                                kit::button(hashed_id("catalog-remove", &item.key()))
                                    .faded_label("Удалить")
                                    .quiet(cx)
                                    .small()
                                    .disabled(
                                        !self.catalog_idle()
                                            || self.catalog_blocked(installed.kind).is_some(),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_catalog_entry(installed.clone(), cx)
                                    }))
                            })),
                    )
                    .child(div().text_sm().child(entry.description.clone()))
                    .children(entry.source.as_ref().map(|source| {
                        let command = source.command();
                        let copied = command.clone();
                        let key = format!("catalog-build-{}-{}", item.source.key(), entry.id);
                        let open = self.is_open(&key);
                        let owner = cx.weak_entity();
                        let toggle_key = key.clone();
                        v_flex()
                            .gap_2()
                            .children((!source.requirements.is_empty()).then(|| {
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "Для сборки: {}",
                                        source.requirements.join(" · ")
                                    ))
                            }))
                            .child(
                                widgets::disclosure(
                                    hashed_id("catalog-build-details", &item.key()),
                                    open,
                                    "Команда сборки",
                                    false,
                                    move |cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.toggle_open(&toggle_key, cx)
                                        });
                                    },
                                    cx,
                                )
                                .px_0()
                                .py_1(),
                            )
                            .children(self.reveal(&key, cx, || {
                                kit::code_surface(cx)
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_start()
                                            .child(div().flex_1().min_w_0().child(command))
                                            .child(
                                                kit::button("catalog-copy-build")
                                                    .icon(IconName::Copy)
                                                    .quiet(cx)
                                                    .xsmall()
                                                    .hint("Копировать команду", cx)
                                                    .on_click(move |_, _, cx| {
                                                        cx.write_to_clipboard(
                                                            ClipboardItem::new_string(
                                                                copied.clone(),
                                                            ),
                                                        )
                                                    }),
                                            ),
                                    )
                                    .into_any_element()
                            }))
                    }))
                    .children(notes.into_iter().flatten().map(|note| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(note)
                    }))
                    .children(
                        item.present
                            .and_then(|package| package.problem.clone())
                            .map(|problem| div().text_xs().text_color(theme.danger).child(problem)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("Источник: {}", item.catalog.title)),
                    )
                    .children(self.catalogs.readme.as_ref().map(|state| {
                        div()
                            .pt_4()
                            .border_t_1()
                            .border_color(theme.border)
                            .child(state.clone())
                    })),
            )
            .overflow_y_scrollbar()
            .into_any_element()
    }
}
