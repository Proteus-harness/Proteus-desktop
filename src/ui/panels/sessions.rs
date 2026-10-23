//! The session list panel: pinned chats, then every project's chats in a
//! folder of its own, with a filter by title.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ui::context::RightClickMenu as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::DropdownMenu as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::{AppSessionActivityStatus, AppSessionSummary};

use super::session_menu;
use crate::sessions::{Project, Row, View};
use crate::ui::chrome::folder_name;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::motion::Spinner;
use crate::ui::tip::Tip as _;
use crate::ui::workspace::Workspace;
use crate::ui::{format, motion, prefs};

/// How long after the list arrives its rows still enter one by one.
const ENTRANCE: Duration = Duration::from_millis(600);
/// Chats a project folder shows before "show more".
const PROJECT_ROWS: usize = 5;
/// Motion key of the archived chats.
const ARCHIVE: &str = "sessions-archive";

impl Workspace {
    pub(super) fn render_sessions(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let current = self.active_chat_dir().map(Path::to_path_buf);
        let cursor = (self.session_list.focus.is_focused(window))
            .then(|| self.session_cursor())
            .flatten();
        let searching = !self.session_list.search.is_empty();
        // A closing archive keeps its rows until they have folded away.
        let archive_moving = !searching && motion::moving(ARCHIVE, self.show_archived, cx);
        let view = View {
            archived: self.show_archived || archive_moving,
            search: &self.session_list.search,
        };
        let listing = prefs::current(cx).sessions.arrange(&self.sessions, &view);
        let mut items: Vec<AnyElement> = Vec::new();
        let mut shown = Vec::new();
        let entering = self
            .sessions_arrived
            .is_some_and(|at| at.elapsed() < ENTRANCE);
        // Rows of a folder that opens or closes go into its block, which is
        // child `slot` of the list.
        let mut rows =
            |rows: &[&Row], indent: bool, items: &mut Vec<AnyElement>, slot: Option<usize>| {
                for row in rows {
                    let dir = &row.summary.session_dir;
                    let ix = shown.len();
                    let look = Look {
                        selected: current.as_ref() == Some(dir),
                        cursor: cursor.as_ref() == Some(dir),
                        indent,
                    };
                    let id = format!("session-enter-{}", dir.display());
                    shown.push((dir.clone(), slot.unwrap_or(items.len())));
                    items.push(
                        motion::stagger_in(
                            div().child(self.render_session_item(ix, row, look, cx)),
                            ElementId::Name(id.into()),
                            ix,
                            entering,
                        )
                        .into_any_element(),
                    );
                }
            };
        if !listing.pinned.is_empty() {
            items.push(heading("Закреплённые", cx).into_any_element());
            rows(
                &listing.pinned.iter().collect::<Vec<_>>(),
                false,
                &mut items,
                None,
            );
        }
        if !listing.projects.is_empty() {
            items.push(heading("Проекты", cx).into_any_element());
        }
        for (gi, project) in listing.projects.iter().enumerate() {
            let open = searching || !self.session_list.collapsed.contains(project.path);
            let all = searching || self.session_list.expanded.contains(project.path);
            let fold = fold_key(project.path);
            let folding = !searching && motion::moving(&fold, open, cx);
            let more = more_key(project.path);
            let more_moving = !searching && motion::moving(&more, all, cx);
            let mut header = Some(
                self.render_project(gi, project, !open, cx)
                    .into_any_element(),
            );
            if !open && !folding {
                items.extend(header);
                continue;
            }
            // A folding folder is one child of the list: its header with
            // the rest in a reveal.
            let slot = folding.then_some(items.len());
            let mut body = Vec::new();
            let target = if folding {
                &mut body
            } else {
                items.extend(header.take());
                &mut items
            };
            let short = |ix: usize, row: &Row| {
                ix < PROJECT_ROWS || current.as_ref() == Some(&row.summary.session_dir)
            };
            let (first, rest): (Vec<_>, Vec<_>) = project
                .rows
                .iter()
                .enumerate()
                .partition(|(ix, row)| short(*ix, row));
            let first: Vec<&Row> = first.into_iter().map(|(_, row)| row).collect();
            let rest: Vec<&Row> = rest.into_iter().map(|(_, row)| row).collect();
            if all && !more_moving {
                let every: Vec<&Row> = project.rows.iter().collect();
                rows(&every, true, target, slot);
            } else {
                rows(&first, true, target, slot);
            }
            if !searching && project.rows.len() > PROJECT_ROWS {
                let hidden = if all { 0 } else { rest.len() };
                let button = self
                    .render_more(gi, project.path, hidden, cx)
                    .into_any_element();
                if more_moving {
                    // The extra chats and the button make one child, so the
                    // block grows from nothing without a gap of its own.
                    let mut extra = Vec::new();
                    rows(&rest, true, &mut extra, Some(slot.unwrap_or(target.len())));
                    let extra =
                        motion::reveal(more, all, cx, || stack(extra).pb_0p5().into_any_element());
                    target.push(v_flex().children(extra).child(button).into_any_element());
                } else {
                    target.push(button);
                }
            }
            if folding {
                let body =
                    motion::reveal(fold, open, cx, || stack(body).pt_0p5().into_any_element());
                items.push(v_flex().children(header).children(body).into_any_element());
            }
        }
        if searching {
            if !listing.archived.is_empty() {
                items.push(heading("Архив", cx).into_any_element());
                rows(
                    &listing.archived.iter().collect::<Vec<_>>(),
                    false,
                    &mut items,
                    None,
                );
            }
        } else if listing.archived_count > 0 {
            let toggle = self
                .render_archive_toggle(listing.archived_count, cx)
                .into_any_element();
            let archived: Vec<&Row> = listing.archived.iter().collect();
            if archive_moving {
                let mut body = Vec::new();
                rows(&archived, false, &mut body, Some(items.len()));
                let body = motion::reveal(ARCHIVE, self.show_archived, cx, || {
                    stack(body).pt_0p5().into_any_element()
                });
                items.push(v_flex().child(toggle).children(body).into_any_element());
            } else {
                items.push(toggle);
                rows(&archived, false, &mut items, None);
            }
        }
        if searching && shown.is_empty() {
            items.push(
                div()
                    .px_2()
                    .py_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Ничего не найдено")
                    .into_any_element(),
            );
        }
        self.session_list.shown = shown;
        let list = v_flex()
            .id("sessions")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.session_list.scroll)
            .px_2()
            .pb_2()
            .gap_0p5()
            .children(items);
        let list = self
            .session_list_actions(v_flex().flex_1().min_h_0().relative(), cx)
            .child(list)
            .vertical_scrollbar(&self.session_list.scroll);
        let filter = div().flex_none().px_2().pb_2().child(
            Input::new(&self.session_list.filter)
                .small()
                .cleanable(true)
                .prefix(Icon::new(IconName::Search).small()),
        );
        self.session_panel_actions(v_flex().size_full(), cx)
            .child(filter)
            .child(list)
    }

    /// A project folder's header: folds its chats and starts a new one there.
    fn render_project(
        &self,
        gi: usize,
        project: &Project,
        collapsed: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let path = project.path.to_path_buf();
        let current = path == self.workspace_path;
        h_flex()
            .id(("session-project", gi))
            .group("project-row")
            .relative()
            .mt_1()
            .px_2()
            .py_1()
            .gap_2()
            .rounded(theme.radius)
            .text_sm()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .hover(|row| row.bg(theme.list_hover).text_color(theme.foreground))
            .child(
                Icon::new(if collapsed {
                    Lucide::Folder
                } else {
                    Lucide::FolderOpen
                })
                .small()
                .flex_none(),
            )
            .child(
                div()
                    .min_w_0()
                    .when(current, |name| name.text_color(theme.foreground))
                    .child(faded(folder_name(&path))),
            )
            // As in Zed's sidebar: the disclosure follows the name on hover.
            .child(
                div()
                    .flex_1()
                    .invisible()
                    .group_hover("project-row", |chevron| chevron.visible())
                    .child(motion::chevron(
                        ("session-project-chevron", gi),
                        !collapsed,
                        theme.muted_foreground,
                    )),
            )
            .child(
                div()
                    .absolute()
                    .right_1()
                    .invisible()
                    .group_hover("project-row", |add| add.visible())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        kit::button(("session-project-new", gi))
                            .icon(Lucide::SquarePen)
                            .ghost()
                            .xsmall()
                            .tip_text("Новый чат в этом проекте")
                            .on_click(cx.listener({
                                let path = path.clone();
                                move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.new_chat_in(path.clone(), window, cx);
                                }
                            })),
                    ),
            )
            .tip_text(path.display().to_string())
            .on_click(cx.listener(move |this, _, _, cx| {
                let collapsed = &mut this.session_list.collapsed;
                let open = collapsed.remove(&path);
                if !open {
                    collapsed.insert(path.clone());
                }
                motion::toggled(fold_key(&path), open, cx);
                cx.notify();
            }))
    }

    fn render_more(
        &self,
        gi: usize,
        path: &Path,
        hidden: usize,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let path: PathBuf = path.to_path_buf();
        div()
            .id(("session-more", gi))
            .pl_7()
            .pr_2()
            .py_1()
            .rounded(theme.radius)
            .text_xs()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .hover(|row| row.text_color(theme.foreground))
            .child(if hidden > 0 {
                format!("Показать больше · {hidden}")
            } else {
                "Свернуть".to_string()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                let expanded = &mut this.session_list.expanded;
                let all = !expanded.remove(&path);
                if all {
                    expanded.insert(path.clone());
                }
                motion::toggled(more_key(&path), all, cx);
                cx.notify();
            }))
    }

    fn render_archive_toggle(&self, count: usize, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .id("sessions-archive")
            .mt_2()
            .px_2()
            .py_1()
            .gap_1()
            .rounded(theme.radius)
            .text_xs()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .hover(|row| row.bg(theme.list_hover))
            .child(motion::chevron(
                "sessions-archive-chevron",
                self.show_archived,
                theme.muted_foreground,
            ))
            .child(format!("Архив · {count}"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_archived = !this.show_archived;
                motion::toggled(ARCHIVE, this.show_archived, cx);
                cx.notify();
            }))
    }

    fn render_session_item(
        &self,
        ix: usize,
        row: &Row,
        look: Look,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let Look {
            selected,
            cursor,
            indent,
        } = look;
        let theme = cx.theme();
        let summary = row.summary;
        let mark = row.mark.clone();
        let dir = summary.session_dir.clone();
        let renaming = self
            .renaming
            .as_ref()
            .filter(|renaming| renaming.dir == dir)
            .map(|renaming| renaming.input.clone());
        let status = summary.activity.as_ref().map(|activity| activity.status);
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
            _ => None,
        };

        let workspace = cx.entity().downgrade();
        let title = mark
            .title
            .clone()
            .unwrap_or_else(|| format::session_title(summary));
        let details = details(summary);
        h_flex()
            .id(("session", ix))
            .group("session-row")
            .relative()
            .px_2()
            .when(indent, |row| row.pl_7())
            .py_1p5()
            .gap_2()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(selected, |row| {
                row.bg(theme.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .when(!selected && cursor, |row| row.bg(theme.list_hover))
            .when(!selected, |row| row.hover(|row| row.bg(theme.list_hover)))
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
                    .child(Input::new(&input).xsmall())
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(faded(title.clone()))
                    .into_any_element(),
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
                    .child(
                        kit::button(("session-menu", ix))
                            .icon(Lucide::Ellipsis)
                            .ghost()
                            .xsmall()
                            .dropdown_menu_with_anchor(Anchor::TopRight, {
                                let (dir, mark) = (dir.clone(), mark.clone());
                                let workspace = workspace.clone();
                                move |menu, _, _| {
                                    session_menu::build(menu, &dir, mark.clone(), &workspace)
                                }
                            }),
                    ),
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
                v_flex()
                    .max_w(prefs::scaled(360.))
                    .gap_0p5()
                    .child(div().text_sm().font_semibold().child(title.clone()))
                    .children(
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
fn details(summary: &AppSessionSummary) -> Vec<String> {
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
    lines.push(format!("Сообщений: {}", summary.message_count));
    if let Some(updated) = summary.updated_at_ms {
        lines.push(format!("Изменена: {}", format::local_time(updated / 1000)));
    }
    lines.push(format!("Папка: {}", summary.workspace_path.display()));
    lines.push(format!("ID: {}", summary.session_id));
    lines
}

/// How a row stands out: the open session, the keyboard cursor; and
/// whether it sits inside a project folder.
#[derive(Clone, Copy)]
struct Look {
    selected: bool,
    cursor: bool,
    indent: bool,
}

/// Motion keys of a project folder's chats and of its chats past the first.
fn fold_key(path: &Path) -> SharedString {
    format!("sessions-fold-{}", path.display()).into()
}

fn more_key(path: &Path) -> SharedString {
    format!("sessions-more-{}", path.display()).into()
}

/// Rows of a moving block, spaced as the list spaces its children.
fn stack(rows: Vec<AnyElement>) -> Div {
    v_flex().gap_0p5().children(rows)
}

fn heading(label: &'static str, cx: &App) -> impl IntoElement {
    div()
        .px_2()
        .pt_2()
        .pb_0p5()
        .text_xs()
        .font_semibold()
        .text_color(cx.theme().muted_foreground)
        .child(label)
}
