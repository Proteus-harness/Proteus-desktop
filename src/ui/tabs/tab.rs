// Адаптировано из Zed: crates/ui/src/components/tab.rs, crates/workspace/src/pane.rs.
//! One tab of a strip: its look, clicks, drag and context menu. Pinned
//! tabs are compact and have no close button.

use crate::ui::context::RightClickMenu as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::menu;
use crate::shortcuts::Command;
use crate::tabs::{self, PaneId, TabLayout};
use crate::ui::chat::ChatState;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::kit::Hint as _;
use crate::ui::motion::Spinner;
use crate::ui::prefs;
use crate::ui::tip::{Tip as _, TooltipAnchor};
use crate::ui::workspace::Workspace;
use crate::ui::{drag, extensions};

pub(in crate::ui) const BAR_HEIGHT: f32 = 38.;
/// Inset the painted surface, keeping the full strip height as the hit target.
const SURFACE_INSET_Y: f32 = 4.;
const SURFACE_INSET_X: f32 = 2.;
const SURFACE_HEIGHT: f32 = BAR_HEIGHT - SURFACE_INSET_Y * 2.;
/// Widest a tab gets, in px at 100% scale.
pub(super) const MAX_WIDTH: f32 = 240.;

/// How narrow a tab gets in a crowded strip before the strip scrolls.
pub(super) fn min_width(pinned: bool) -> Rems {
    if pinned {
        prefs::scaled(56.)
    } else {
        prefs::scaled(88.)
    }
}

/// Payload of a tab being dragged; the preview is a lifted copy of the tab.
#[derive(Clone)]
pub(in crate::ui) struct DraggedTab {
    pub id: String,
    icon: Icon,
    title: SharedString,
    preview: bool,
}

impl Render for DraggedTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let foreground = cx.theme().tab_active_foreground;
        let style = window.text_style();
        let font_size = prefs::scaled(14.).to_pixels(window.rem_size());
        let width = window
            .text_system()
            .shape_line(
                self.title.clone(),
                font_size,
                &[style.to_run(self.title.len())],
                None,
            )
            .width();
        let width = (width + prefs::scaled(64.).to_pixels(window.rem_size())).clamp(
            prefs::scaled(120.).to_pixels(window.rem_size()),
            prefs::scaled(MAX_WIDTH).to_pixels(window.rem_size()),
        );
        drag::lifted(
            h_flex()
                .h(prefs::scaled(SURFACE_HEIGHT))
                .w(width)
                .min_w(width)
                .flex_none()
                .px_3()
                .gap_2()
                .text_sm()
                .text_color(foreground)
                .child(self.icon.clone().small())
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .when(self.preview, |title| title.italic())
                        .child(faded(self.title.clone())),
                ),
            cx,
        )
    }
}

struct TabLabel {
    icon: Icon,
    title: SharedString,
    tooltip: Option<SharedString>,
}

/// How a tab sits in its strip.
#[derive(Clone, Copy)]
pub(super) struct TabState {
    pub active: bool,
    pub focused: bool,
    pub preview: bool,
    pub pinned: bool,
}

impl Workspace {
    #[cfg(test)]
    pub(in crate::ui) fn test_tab_title(&self, id: &str, cx: &App) -> SharedString {
        self.tab_label(id, cx).title
    }

    fn chat_state(&self, id: &str, cx: &App) -> Option<ChatState> {
        Some(self.chats.get(id)?.view.read(cx).session.state())
    }

    fn tab_label(&self, id: &str, cx: &App) -> TabLabel {
        if let Some(target) = super::super::terminal::Target::parse(id) {
            return TabLabel {
                icon: Icon::new(Lucide::Terminal),
                title: target.terminal.title.into(),
                tooltip: Some("Терминал команды агента".into()),
            };
        }
        if super::super::chat::readonly::Target::parse(id).is_some() {
            return TabLabel {
                icon: Icon::new(Lucide::MessageSquare),
                title: "Чат субагента".into(),
                tooltip: Some("Сохранённая история · только просмотр".into()),
            };
        }
        if let Some(key) = tabs::chat_key(id) {
            let tooltip = match self.chat_state(id, cx) {
                Some(ChatState::Busy(text) | ChatState::Trouble(text)) => Some(text.into()),
                _ => None,
            };
            return TabLabel {
                icon: Icon::new(Lucide::MessageSquare),
                title: self.session_title(&key, cx).into(),
                tooltip,
            };
        }
        match extensions::document(id, cx) {
            Some((manifest, argument)) => TabLabel {
                icon: Icon::new(IconName::File),
                title: self.document_titles.get(id).cloned().unwrap_or_else(|| {
                    manifest
                        .document
                        .as_ref()
                        .map_or_else(|| argument.clone().into(), |doc| doc.title(&argument))
                }),
                tooltip: std::path::Path::new(&argument)
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .map(|_| argument.clone().into()),
            },
            None => TabLabel {
                icon: Icon::new(IconName::File),
                title: id.to_string().into(),
                tooltip: None,
            },
        }
    }

    pub(super) fn render_tab(
        &self,
        pane: PaneId,
        id: &str,
        state: TabState,
        layout: &TabLayout,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let label = self.tab_label(id, cx);
        let tab_group = SharedString::from(format!("tab-{id}"));
        let hover = theme.list_hover;
        let foreground = crate::ui::selection::tab_foreground(theme, state.active);
        let lifted = drag::is_source(&format!("tab:{id}"), cx);
        let icon = match self.chat_state(id, cx) {
            Some(ChatState::Busy(_)) => Spinner::new().xsmall().into_any_element(),
            Some(ChatState::Trouble(_)) => Icon::new(IconName::TriangleAlert)
                .small()
                .text_color(theme.warning)
                .into_any_element(),
            _ => label.icon.clone().small().into_any_element(),
        };
        let dragged = DraggedTab {
            id: id.to_string(),
            icon: label.icon.clone(),
            title: label.title.clone(),
            preview: state.preview,
        };
        let tooltip = label.tooltip.clone();
        let anchor = TooltipAnchor::default();
        let menu = menu::TabMenu::new(pane, id, state.pinned, layout);
        let (click_id, close_id, middle_id) = (id.to_string(), id.to_string(), id.to_string());
        let workspace = cx.weak_entity();
        let close_keys = state
            .active
            .then(|| kit::command_keys(Command::CloseTab, cx))
            .flatten();
        h_flex()
            .id(ElementId::Name(format!("tab-{id}").into()))
            .group(tab_group.clone())
            .relative()
            .tip_anchor(anchor.clone())
            .h_full()
            .flex_none()
            .map(|tab| match state.pinned {
                true => tab.max_w(prefs::scaled(140.)).px_3().gap_2(),
                false => tab
                    .max_w(prefs::scaled(MAX_WIDTH))
                    .pl_3()
                    .pr(prefs::scaled(4.))
                    .gap_2(),
            })
            .text_sm()
            .cursor_pointer()
            .text_color(foreground)
            .when(lifted, |tab| tab.opacity(drag::SOURCE_OPACITY))
            .child(
                div()
                    .absolute()
                    .top(prefs::scaled(SURFACE_INSET_Y))
                    .bottom(prefs::scaled(SURFACE_INSET_Y))
                    .left(prefs::scaled(SURFACE_INSET_X))
                    .right(prefs::scaled(SURFACE_INSET_X))
                    .rounded(theme.radius)
                    .when(state.active, |surface| surface.bg(theme.tab_active))
                    .when(!state.active, |surface| {
                        surface.group_hover(tab_group.clone(), move |surface| surface.bg(hover))
                    }),
            )
            .child(icon)
            .child(
                div()
                    .min_w_0()
                    .when(state.active && state.focused, |title| title.font_medium())
                    .when(state.preview, |title| title.italic())
                    .child(crate::ui::fade::caption("tab-caption", label.title).anchor(anchor)),
            )
            .when(!state.pinned, |tab| {
                tab.child(
                    div()
                        // Closing a background tab must not activate its pane
                        // or take focus from an editor in another pane.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when(!state.active, |close| {
                            close
                                .invisible()
                                .group_hover(tab_group, |style| style.visible())
                        })
                        .child(
                            kit::button(ElementId::Name(format!("tab-close-{id}").into()))
                                .quiet(cx)
                                .xsmall()
                                .icon(IconName::Close)
                                .hint_keys("Закрыть", close_keys.as_deref(), cx)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_tab(&close_id, window, cx);
                                })),
                        ),
                )
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| this.close_tab(&middle_id, window, cx)),
                )
            })
            .when_some(tooltip, |tab, text| tab.tip_text(text))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() > 1 {
                    this.keep_tab(&click_id, cx);
                } else {
                    this.activate_tab(pane, &click_id, window, cx);
                }
            }))
            .on_drag(dragged, {
                let workspace = cx.weak_entity();
                move |dragged, _, _, cx| {
                    drag::start(format!("tab:{}", dragged.id), cx);
                    workspace
                        .update(cx, |this, cx| {
                            this.tab_drag = true;
                            cx.notify();
                        })
                        .ok();
                    cx.new(|_| dragged.clone())
                }
            })
            .right_click_menu(move |popup, _, _| menu.build(popup, &workspace))
            .trigger_style(StyleRefinement::default().h_full().flex_none())
    }
}
