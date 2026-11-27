//! A saved conversation row and its disclosure, activity and context menu.
// Адаптировано из Zed: crates/project_panel/src/project_panel.rs

use std::rc::Rc;

use super::{
    session_menu,
    sessions::{OrderGroup, branch_key},
};
use crate::conversations::ConversationSummary;
use crate::sessions::SessionMark;
use crate::ui::context::RightClickMenu as _;
use crate::ui::kit;
use crate::ui::kit::{ButtonLabel as _, Hint as _};
use crate::ui::motion::Spinner;
use crate::ui::tip::{Tip as _, TooltipAnchor};
use crate::ui::workspace::Workspace;
use crate::ui::{format, motion, prefs, reorder};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppSessionActivityStatus;

/// How a row stands out: the open session, the keyboard cursor; and
/// whether it sits inside a project folder.
#[derive(Clone, Copy)]
pub(super) struct Look {
    pub(super) selected: bool,
    pub(super) cursor: bool,
    pub(super) depth: usize,
    pub(super) branch_open: bool,
    pub(super) branch_fits: bool,
}

impl Workspace {
    pub(super) fn render_session_item(
        &self,
        ix: usize,
        summary: &ConversationSummary,
        mark: SessionMark,
        has_children: bool,
        look: Look,
        order_group: Option<Rc<OrderGroup>>,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let Look {
            selected,
            cursor,
            depth,
            branch_open,
            branch_fits,
        } = look;
        let theme = cx.theme();
        let dir = summary.key.clone();
        let renaming = self
            .renaming
            .as_ref()
            .filter(|renaming| renaming.key == dir)
            .map(|renaming| renaming.input.clone());
        let order_group = order_group.filter(|_| self.renaming.is_none());
        let status = summary.activity.as_ref().map(|activity| activity.status);
        let live = self
            .chats
            .get(&dir.tab_id())
            .map(|chat| chat.view.read(cx).session.state());
        let indicator = match status {
            Some(AppSessionActivityStatus::Running) => {
                Some(Spinner::new().xsmall().into_any_element())
            }
            Some(
                AppSessionActivityStatus::WaitingApproval | AppSessionActivityStatus::WaitingInput,
            ) => Some(
                Icon::new(IconName::CircleAlert)
                    .xsmall()
                    .tone(IconTone::Warning)
                    .into_any_element(),
            ),
            _ if matches!(live, Some(crate::ui::chat::ChatState::Busy(_))) => {
                Some(Spinner::new().xsmall().into_any_element())
            }
            _ => None,
        };

        let workspace = cx.entity().downgrade();
        let title = mark
            .title
            .clone()
            .unwrap_or_else(|| format::session_title(summary));
        let title: SharedString = title.into();
        let mut details = details(summary);
        let last_change = summary.updated_at_ms;
        if matches!(dir, crate::tabs::ChatKey::Codex(_)) {
            details[0] = match live {
                Some(
                    crate::ui::chat::ChatState::Busy(text)
                    | crate::ui::chat::ChatState::Trouble(text),
                ) => format!("Codex: {text}"),
                _ => "Агент: Codex".into(),
            };
        }
        let anchor = TooltipAnchor::default();
        let caption_height = kit::compact_row_line_height();
        let hover_background = theme.sidebar.blend(if selected {
            *theme.tokens.sidebar_accent
        } else {
            *theme.tokens.list_hover
        });
        let actions = || {
            let pinned = mark.pinned;
            let pin = kit::button(("session-pin", ix))
                .icon(if pinned { Lucide::PinOff } else { Lucide::Pin })
                .quiet(cx)
                .xsmall()
                .hint(session_menu::pin_label(pinned), cx)
                .on_click({
                    let dir = dir.clone();
                    move |_, _, cx| {
                        cx.stop_propagation();
                        prefs::update(cx, |p| p.sessions.set_pinned(&dir, !pinned));
                    }
                });
            let menu = crate::ui::context::menu_button(
                ("session-dropdown", ix),
                kit::button(("session-menu", ix))
                    .icon(Lucide::Ellipsis)
                    .quiet(cx)
                    .xsmall(),
                false,
                {
                    let (dir, mark) = (dir.clone(), mark.clone());
                    let workspace = workspace.clone();
                    move |menu, _, _| session_menu::build(menu, &dir, mark.clone(), &workspace)
                },
            );
            h_flex()
                .absolute()
                .right_0()
                .top_0()
                .bottom_0()
                .invisible()
                .group_hover("session-row", |actions| actions.visible())
                // Cover the caption's tail without changing its layout.
                .child(div().w_4().h_full().bg(linear_gradient(
                    90.,
                    linear_color_stop(hover_background.opacity(0.), 0.),
                    linear_color_stop(hover_background, 1.),
                )))
                .child(
                    h_flex()
                        .h_full()
                        .gap_0p5()
                        .bg(hover_background)
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(pin)
                        .child(menu),
                )
        };
        let row = h_flex()
            .id(("session", ix))
            .tip_anchor(anchor.clone())
            .group("session-row")
            .relative()
            .pr_1()
            .pl(rems(0.5 + depth.min(12) as f32 * 1.25))
            .h(kit::compact_row_height())
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .child(crate::ui::selection::background(
                selected,
                !selected && cursor,
                "session-row",
                false,
                cx,
            ))
            .when(mark.archived, |row| row.text_color(theme.muted_foreground))
            .when(mark.pinned, |row| {
                row.child(
                    Icon::new(Lucide::Pin)
                        .xsmall()
                        .flex_none()
                        .tone(IconTone::Muted),
                )
            })
            .child(match renaming {
                Some(input) => div()
                    .flex_1()
                    .min_w_0()
                    .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" {
                            cx.stop_propagation();
                            this.finish_rename(false, cx);
                            this.session_list.focus.focus(window, cx);
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(Styled::h(
                        // Input::h is only used for multiline inputs.
                        kit::input(&input)
                            .appearance(false)
                            .aria_label("Название сессии")
                            .text_sm()
                            .line_height(caption_height)
                            .p_0(),
                        caption_height,
                    ))
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .relative()
                    .text_sm()
                    .line_height(caption_height)
                    .child(
                        crate::ui::fade::caption("session-caption", title.clone()).anchor(anchor),
                    )
                    .child(actions())
                    .into_any_element(),
            })
            .when(has_children, |item| {
                let branch = dir.clone();
                item.child(
                    div()
                        .id(("session-branch-toggle", ix))
                        .flex_none()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(motion::chevron(
                            ("session-branch-chevron", ix),
                            branch_open,
                            theme.muted_foreground,
                            cx,
                        ))
                        .tip_text("Дочерние сессии")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let open = !this.session_list.branches.remove(&branch);
                            if open {
                                this.session_list.branches.insert(branch.clone());
                            }
                            if branch_fits {
                                motion::toggled(branch_key(&branch), open, cx);
                            }
                            cx.notify();
                        })),
                )
            })
            .children(indicator)
            .on_click(cx.listener({
                let dir = dir.clone();
                move |this, event: &ClickEvent, window, cx| {
                    this.session_list.cursor = Some(dir.clone());
                    this.session_list.focus.focus(window, cx);
                    this.select_session(dir.clone(), event.click_count(), window, cx)
                }
            }))
            .tip_element(move |_, cx| {
                let muted = cx.theme().muted_foreground;
                let last_change = last_change.map(|timestamp| {
                    format!("Последнее действие: {}", format::relative_age(timestamp))
                });
                v_flex().max_w(prefs::scaled(360.)).gap_0p5().children(
                    last_change
                        .iter()
                        .chain(&details)
                        .map(|line| div().text_xs().text_color(muted).child(line.clone())),
                )
            });
        let row = if let Some(group) = order_group {
            let owner = cx.entity_id();
            let id = dir.tab_id();
            let source_key = format!("session-order:{owner:?}:{id}").into();
            let target = dir.clone();
            let workspace = workspace.clone();
            reorder::virtual_row(
                row,
                (owner, group),
                id.into(),
                source_key,
                title,
                move |source, after, cx| {
                    let Some(source) = crate::tabs::chat_key(source.as_ref()) else {
                        return;
                    };
                    let _ = workspace.update(cx, |this, cx| {
                        if this.session_list.search.is_empty() && this.renaming.is_none() {
                            prefs::update(cx, |prefs| {
                                prefs
                                    .sessions
                                    .reorder(&this.sessions, &source, &target, after)
                            });
                        }
                    });
                },
                cx,
            )
        } else {
            row
        };
        row.right_click_menu(move |menu, _, _| {
            session_menu::build(menu, &dir, mark.clone(), &workspace)
        })
        .trigger_style(StyleRefinement::default().w_full())
    }
}

/// What the tooltip of a session row tells beside its title.
fn details(summary: &ConversationSummary) -> Vec<String> {
    let activity = summary.activity.as_ref();
    let status = match activity.map(|activity| activity.status) {
        Some(AppSessionActivityStatus::Running) => "работает",
        Some(AppSessionActivityStatus::WaitingApproval) => "ждёт подтверждения",
        Some(AppSessionActivityStatus::WaitingInput) => "ждёт ответа",
        _ => "ожидает",
    };
    let mut lines = vec![format!("Состояние: {status}")];
    if let Some(activity) = activity.filter(|activity| activity.running_runs > 1) {
        lines.push(format!("Запусков: {}", activity.running_runs));
    }
    if matches!(summary.key, crate::tabs::ChatKey::Proteus(_)) {
        lines.push(format!("Сообщений: {}", summary.message_count));
    }
    lines.push(format!("Папка: {}", summary.workspace_path.display()));
    lines.push(format!("ID: {}", summary.key));
    lines
}
