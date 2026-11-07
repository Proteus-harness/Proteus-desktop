// Адаптировано из Zed: crates/extensions_ui/src/extensions_ui.rs (shared header/filter gutters).
//! Catalog title and filters share one content axis and control sizes.
use super::items::{KINDS, kind_filter, kind_icon};
use super::{SettingsView, Show};
use crate::settings::catalog::{PageId, page_description, page_title};
use crate::ui::kit::{ButtonLabel as _, Hint as _};
use crate::ui::{kit, prefs};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

impl SettingsView {
    pub(super) fn catalog_header(
        &self,
        sources: usize,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let loading = !self.catalogs.loading.is_empty();
        h_flex()
            .px_6()
            .pt_5()
            .pb_4()
            .gap_4()
            .items_center()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_semibold()
                            .child(page_title(self.snapshot(), &PageId::Catalog)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(page_description(self.snapshot(), &PageId::Catalog)),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .child(
                        kit::button("catalog-refresh-all")
                            .icon(Icon::new(Lucide::RefreshCw))
                            .quiet(cx)
                            .small()
                            .loading(loading)
                            .loading_icon(IconName::LoaderCircle)
                            .disabled(loading || sources == 0 || !self.catalog_idle())
                            .hint("Скачать источники заново", cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let sources = prefs::current(cx).catalogs.sources.clone();
                                this.refresh_catalog_inventory(cx);
                                this.refresh_catalogs(sources, cx)
                            })),
                    )
                    .child(
                        kit::button("catalog-sources")
                            .icon(Icon::new(Lucide::GitBranch))
                            .faded_label(if sources == 0 {
                                "Источники".to_string()
                            } else {
                                format!("Источники · {sources}")
                            })
                            .outline()
                            .small()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_catalog_sources(window, cx)
                            })),
                    ),
            )
    }

    pub(super) fn catalog_toolbar(
        &self,
        updates: usize,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let show = |id: &'static str, show: Show, label: String| {
            kit::button(id)
                .faded_label(label)
                .quiet(cx)
                .small()
                .selected(self.catalogs.show == show)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.catalogs.show = show;
                    cx.notify();
                }))
        };
        let kind = |id: ElementId, kind: Option<_>, label: &'static str| {
            kit::button(id)
                .faded_label(label)
                .when_some(kind, |button, kind| button.icon(kind_icon(kind)))
                .quiet(cx)
                .xsmall()
                .selected(self.catalogs.kind == kind)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.catalogs.kind = kind;
                    cx.notify();
                }))
        };
        v_flex()
            .px_6()
            .pb_3()
            .gap_2()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        Self::catalog_search_keys(div()).flex_1().min_w_0().child(
                            crate::ui::search::field(&self.catalogs.search)
                                .small()
                                .cleanable(true),
                        ),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .child(show(
                                "catalog-show-overview",
                                Show::Overview,
                                "Обзор".into(),
                            ))
                            .child(show("catalog-show-all", Show::All, "Все".into()))
                            .child(show(
                                "catalog-show-installed",
                                Show::Installed,
                                "Установленные".into(),
                            ))
                            .child(show(
                                "catalog-show-updates",
                                Show::Updates,
                                if updates == 0 {
                                    "Обновления".into()
                                } else {
                                    format!("Обновления · {updates}")
                                },
                            )),
                    ),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child(kind("catalog-kind-all".into(), None, "Все типы"))
                    .children(KINDS.into_iter().map(|item| {
                        kind(
                            ElementId::Name(format!("catalog-kind-{item:?}").into()),
                            Some(item),
                            kind_filter(item),
                        )
                    }))
                    .children(self.catalogs.scope.as_ref().map(|scope| {
                        kit::button("catalog-scope")
                            .faded_label(self.catalog_scope_title(scope))
                            .icon(Icon::new(IconName::Close))
                            .outline()
                            .xsmall()
                            .ml_2()
                            .hint("Показать все источники", cx)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.catalogs.scope = None;
                                cx.notify();
                            }))
                    })),
            )
    }
}
