//! A saved conversation row and its disclosure, activity and context menu.
// Адаптировано из Zed: crates/project_panel/src/project_panel.rs

use super::{session_menu, sessions::branch_key};
use crate::conversations::ConversationSummary;
use crate::sessions::Row;
use crate::ui::context::RightClickMenu as _;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::motion::Spinner;
use crate::ui::tip::{Tip as _, TooltipAnchor};
use crate::ui::workspace::Workspace;
use crate::ui::{format, motion, prefs};
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
}

impl Workspace {
    pub(super) fn render_session_item(
        &self,
        ix: usize,
        row: &Row,
        look: Look,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let Look {
            selected,
            cursor,
            depth,
            branch_open,
        } = look;
        let theme = cx.theme();
        let summary = row.summary;
        let mark = row.mark.clone();
        let dir = summary.key.clone();
        let renaming = self
            .renaming
            .as_ref()
            .filter(|renaming| renaming.key == dir)
            .map(|renaming| renaming.input.clone());
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
                    .text_color(theme.warning)
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
        let mut details = details(summary);
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
        h_flex()
            .id(("session", ix))
            .tip_anchor(anchor.clone())
            .group("session-row")
            .relative()
            .px_2()
            .pl(rems(0.5 + depth.min(12) as f32 * 1.25))
            .py_1p5()
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
            .when(selected, |row| {
                row.text_color(theme.sidebar_accent_foreground)
            })
            .when(mark.archived && !selected, |row| {
                row.text_color(theme.muted_foreground)
            })
            .when(mark.pinned, |row| {
                row.child(
                    Icon::new(Lucide::Pin)
                        .xsmall()
                        .flex_none()
                        .text_color(theme.muted_foreground),
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
                    .child(kit::input(&input).xsmall())
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(crate::ui::fade::caption("session-caption", title).anchor(anchor))
                    .into_any_element(),
            })
            .when(!row.children.is_empty(), |item| {
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
                        ))
                        .tip_text("Дочерние сессии")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            let open = !this.session_list.branches.remove(&branch);
                            if open {
                                this.session_list.branches.insert(branch.clone());
                            }
                            motion::toggled(branch_key(&branch), open, cx);
                            cx.notify();
                        })),
                )
            })
            .children(indicator)
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .group_hover("session-row", |time| time.invisible())
                    .child(format::relative_time(summary.updated_at_ms)),
            )
            .child(
                div()
                    .absolute()
                    .right_1()
                    .invisible()
                    .group_hover("session-row", |menu| menu.visible())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(crate::ui::context::menu_button(
                        ("session-dropdown", ix),
                        kit::button(("session-menu", ix))
                            .icon(Lucide::Ellipsis)
                            .quiet(cx)
                            .xsmall(),
                        false,
                        {
                            let (dir, mark) = (dir.clone(), mark.clone());
                            let workspace = workspace.clone();
                            move |menu, _, _| {
                                session_menu::build(menu, &dir, mark.clone(), &workspace)
                            }
                        },
                    )),
            )
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
                v_flex().max_w(prefs::scaled(360.)).gap_0p5().children(
                    details
                        .iter()
                        .map(|line| div().text_xs().text_color(muted).child(line.clone())),
                )
            })
            .right_click_menu(move |menu, _, _| {
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
    if let Some(updated) = summary.updated_at_ms {
        lines.push(format!("Изменена: {}", format::local_time(updated / 1000)));
    }
    lines.push(format!("Папка: {}", summary.workspace_path.display()));
    lines.push(format!("ID: {}", summary.key));
    lines
}
