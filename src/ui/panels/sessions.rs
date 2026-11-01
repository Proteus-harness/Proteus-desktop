//! The session list panel: pinned chats, then every project's chats in a
//! folder of its own, with a filter by title.

use proteus_tabs::ChatKey;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::scroll::{ScrollableElement as _, vertical_scroll_gutter};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::session_row::Look;
use crate::sessions::{Project, Row, View};
use crate::ui::chrome::folder_name;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::tip::Tip as _;
use crate::ui::workspace::Workspace;
use crate::ui::{motion, prefs};

/// How long after the list arrives its rows still enter one by one.
const ENTRANCE: Duration = Duration::from_millis(600);
/// Chats a project folder shows before "show more".
const PROJECT_ROWS: usize = 5;
/// Motion key of the archived chats.
const ARCHIVE: &str = "sessions-archive";

impl Workspace {
    pub(super) fn render_sessions(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let current = self.active_chat_key();
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
                    self.render_session_branch(
                        row,
                        usize::from(indent),
                        current.as_ref(),
                        cursor.as_ref(),
                        searching,
                        entering,
                        items,
                        slot,
                        &mut shown,
                        cx,
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
                ix < PROJECT_ROWS || current.as_ref().is_some_and(|dir| row.contains(dir))
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
        let list =
            vertical_scroll_gutter(list, &self.session_list.scroll, rems(0.5), rems(1.), window);
        let list = self
            .session_list_actions(v_flex().flex_1().min_h_0().relative(), cx)
            .child(list)
            .vertical_scrollbar(&self.session_list.scroll);
        v_flex().size_full().child(list)
    }

    /// A branch is one moving block, like project/file trees in Zed. Merely
    /// revealing summaries never creates a Chat or starts an event stream.
    #[allow(clippy::too_many_arguments)]
    fn render_session_branch(
        &self,
        row: &Row,
        depth: usize,
        current: Option<&ChatKey>,
        cursor: Option<&ChatKey>,
        searching: bool,
        entering: bool,
        items: &mut Vec<AnyElement>,
        slot: Option<usize>,
        shown: &mut Vec<(ChatKey, usize)>,
        cx: &Context<Self>,
    ) {
        let dir = &row.summary.key;
        let open = searching
            || self.session_list.branches.contains(dir)
            || current.is_some_and(|current| current != dir && row.contains(current));
        let key = branch_key(dir);
        let moving = !searching && motion::moving(&key, open, cx);
        let index = shown.len();
        let child_slot = slot.unwrap_or(items.len());
        shown.push((dir.clone(), child_slot));
        let look = Look {
            selected: current == Some(dir),
            cursor: cursor == Some(dir),
            depth,
            branch_open: open,
        };
        let item = motion::stagger_in(
            div().child(self.render_session_item(index, row, look, cx)),
            ElementId::Name(format!("session-enter-{}", dir).into()),
            index,
            entering,
        )
        .into_any_element();
        if row.children.is_empty() || (!open && !moving) {
            items.push(item);
            return;
        }
        if moving {
            let mut children = Vec::new();
            for row in &row.children {
                self.render_session_branch(
                    row,
                    depth + 1,
                    current,
                    cursor,
                    searching,
                    entering,
                    &mut children,
                    Some(child_slot),
                    shown,
                    cx,
                );
            }
            let body = motion::reveal(key, open, cx, || {
                stack(children).pt_0p5().into_any_element()
            });
            items.push(v_flex().child(item).children(body).into_any_element());
        } else {
            items.push(item);
            for row in &row.children {
                self.render_session_branch(
                    row,
                    depth + 1,
                    current,
                    cursor,
                    searching,
                    entering,
                    items,
                    slot,
                    shown,
                    cx,
                );
            }
        }
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
        let current = path == self.active_project_path();
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
                        h_flex()
                            .gap_0p5()
                            .child(
                                kit::button(("session-project-open", gi))
                                    .icon(Lucide::FolderOpen)
                                    .quiet(cx)
                                    .xsmall()
                                    .tip_text("Открыть проект")
                                    .on_click(cx.listener({
                                        let path = path.clone();
                                        move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.open_project(path.clone(), window, cx);
                                        }
                                    })),
                            )
                            .child(
                                kit::button(("session-project-new", gi))
                                    .icon(Lucide::SquarePen)
                                    .quiet(cx)
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
}

pub(super) fn branch_key(path: &ChatKey) -> SharedString {
    format!("session-branch-{path}").into()
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
