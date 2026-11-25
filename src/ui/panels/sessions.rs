//! The session list panel: pinned chats, then every project's chats in a
//! folder of its own, with a filter by title.

use proteus_tabs::ChatKey;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::session_list::LayoutSlot;
use super::session_row::Look;
use crate::sessions::{Row, View};
use crate::ui::chrome::folder_name;
use crate::ui::fade::faded;
use crate::ui::kit;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::tip::Tip as _;
use crate::ui::workspace::Workspace;
use crate::ui::{motion, prefs};

mod project_rows;
use project_rows::PROJECT_ROWS;

/// How long after the list arrives its rows still enter one by one.
const ENTRANCE: Duration = Duration::from_millis(600);
/// Motion key of the archived chats.
const ARCHIVE: &str = "sessions-archive";

#[derive(Clone, PartialEq, Eq)]
pub(super) enum ItemId {
    Heading(&'static str),
    Project(Rc<Path>),
    More(Rc<Path>),
    Archive,
    Session(Rc<ChatKey>),
    Empty,
}

struct Item {
    content: Content,
    fold: Option<Fold>,
}

enum Content {
    Heading(&'static str),
    Project {
        path: Rc<Path>,
        index: usize,
        open: bool,
        animate: bool,
    },
    More {
        path: Rc<Path>,
        index: usize,
        hidden: usize,
        all: bool,
        animate: bool,
    },
    Archive {
        count: usize,
        animate: bool,
    },
    Session {
        key: Rc<ChatKey>,
        index: usize,
        ordinal: usize,
        look: Look,
        has_children: bool,
    },
    Empty,
}

struct Fold {
    key: SharedString,
    open: bool,
    before: bool,
    body: Vec<Item>,
}

impl Item {
    fn new(content: Content) -> Self {
        Self {
            content,
            fold: None,
        }
    }

    fn id(&self) -> ItemId {
        match &self.content {
            Content::Heading(label) => ItemId::Heading(label),
            Content::Project { path, .. } => ItemId::Project(path.clone()),
            Content::More { path, .. } => ItemId::More(path.clone()),
            Content::Archive { .. } => ItemId::Archive,
            Content::Session { key, .. } => ItemId::Session(key.clone()),
            Content::Empty => ItemId::Empty,
        }
    }

    fn weight(&self) -> usize {
        1 + self
            .fold
            .as_ref()
            .map_or(0, |fold| fold.body.iter().map(Self::weight).sum())
    }

    fn cursor_targets(&self, slot: usize, shown: &mut Vec<(Rc<ChatKey>, usize)>) {
        let visit = |shown: &mut Vec<_>| {
            if let Some(fold) = &self.fold {
                for item in &fold.body {
                    item.cursor_targets(slot, shown);
                }
            }
        };
        let before = self.fold.as_ref().is_some_and(|fold| fold.before);
        if before {
            visit(shown);
        }
        if let Content::Session { key, .. } = &self.content {
            shown.push((key.clone(), slot));
        }
        if !before {
            visit(shown);
        }
    }
}

impl Fold {
    /// Keep large branches as individual list items instead of laying out
    /// every descendant in one animated item at each frame.
    fn append(self, mut head: Item, list: &mut Vec<Item>, limit: usize) {
        let fits = !self.body.is_empty()
            && self.body.len() <= limit
            && self.body.iter().map(Item::weight).sum::<usize>() <= limit;
        if fits {
            head.fold = Some(self);
            list.push(head);
            return;
        }
        if self.before && self.open {
            list.extend(self.body);
            list.push(head);
        } else {
            list.push(head);
            if self.open {
                list.extend(self.body);
            }
        }
    }
}

struct SessionLayout<'a> {
    current: Option<&'a ChatKey>,
    cursor: Option<&'a ChatKey>,
    branches: &'a std::collections::HashSet<ChatKey>,
    searching: bool,
    at: Instant,
    limit: usize,
    ordinal: usize,
}

impl SessionLayout<'_> {
    fn add(&mut self, row: &Row, depth: usize, items: &mut Vec<Item>, cx: &App) {
        let dir = &row.summary.key;
        let open = self.searching
            || self.branches.contains(dir)
            || self
                .current
                .is_some_and(|current| current != dir && row.contains(current));
        let branch_fits = row.children.len() <= self.limit;
        let item = Item::new(Content::Session {
            key: Rc::new(dir.clone()),
            index: row.index,
            ordinal: self.ordinal,
            look: Look {
                selected: self.current == Some(dir),
                cursor: self.cursor == Some(dir),
                depth,
                branch_open: open,
                branch_fits,
            },
            has_children: !row.children.is_empty(),
        });
        self.ordinal += 1;
        if !row.children.is_empty()
            && branch_fits
            && let key = branch_key(dir)
            && self.moving(&key, open, cx)
        {
            let mut body = Vec::new();
            self.add_all(&row.children, depth + 1, &mut body, cx);
            Fold {
                key,
                open,
                before: false,
                body,
            }
            .append(item, items, self.limit);
        } else {
            items.push(item);
            if open {
                self.add_all(&row.children, depth + 1, items, cx);
            }
        }
    }

    fn add_all(&mut self, rows: &[Row], depth: usize, items: &mut Vec<Item>, cx: &App) {
        for row in rows {
            self.add(row, depth, items, cx);
        }
    }

    fn moving(&self, key: &str, open: bool, cx: &App) -> bool {
        !self.searching && motion::moving_at(key, open, self.at, cx)
    }
}

impl Workspace {
    pub(super) fn render_sessions(&mut self, window: &Window, cx: &mut Context<Self>) -> Div {
        let current = self.active_chat_key();
        let cursor = self
            .session_list
            .focus
            .is_focused(window)
            .then(|| self.session_cursor())
            .flatten();
        let at = Instant::now();
        let mut layout = SessionLayout {
            current: current.as_ref(),
            cursor: cursor.as_ref(),
            branches: &self.session_list.branches,
            searching: !self.session_list.search.is_empty(),
            at,
            limit: self.session_list.fold_limit(window),
            ordinal: 0,
        };
        // A closing archive keeps its rows until the reveal ends.
        let archive_moving = layout.moving(ARCHIVE, self.show_archived, cx);
        let listing = prefs::current(cx).sessions.arrange(
            &self.sessions,
            &View {
                archived: self.show_archived || archive_moving,
                search: &self.session_list.search,
            },
        );
        let mut items = Vec::new();
        if !listing.pinned.is_empty() {
            items.push(Item::new(Content::Heading("Закреплённые")));
            layout.add_all(&listing.pinned, 0, &mut items, cx);
        }
        if !listing.projects.is_empty() {
            items.push(Item::new(Content::Heading("Проекты")));
        }
        for (index, project) in listing.projects.iter().enumerate() {
            let path: Rc<Path> = project.path.into();
            let open = layout.searching || !self.session_list.collapsed.contains(project.path);
            let all = layout.searching || self.session_list.expanded.contains(project.path);
            let animate = !all || project.rows.len() <= layout.limit;
            let head = Item::new(Content::Project {
                path: path.clone(),
                index,
                open,
                animate,
            });
            let key = fold_key(project.path);
            let folding = animate && layout.moving(&key, open, cx);
            if !open && !folding {
                items.push(head);
                continue;
            }
            let mut body = Vec::new();
            let more = more_key(project.path);
            let animate_more = project.rows.len().saturating_sub(PROJECT_ROWS) <= layout.limit;
            let more_moving = animate_more && layout.moving(&more, all, cx);
            let (hidden, rest) =
                project_rows::visit(&project.rows, current.as_ref(), all, more_moving, |row| {
                    layout.add(row, 1, &mut body, cx)
                });
            let has_more = self
                .codex_catalogs
                .get(project.path)
                .is_some_and(|cursor| cursor.has_more());
            if !layout.searching && (project.rows.len() > PROJECT_ROWS || has_more) {
                let button = Item::new(Content::More {
                    path,
                    index,
                    hidden,
                    all,
                    animate: animate_more,
                });
                if more_moving {
                    let mut extra = Vec::new();
                    for row in rest {
                        layout.add(row, 1, &mut extra, cx);
                    }
                    Fold {
                        key: more,
                        open: all,
                        before: true,
                        body: extra,
                    }
                    .append(button, &mut body, layout.limit);
                } else {
                    body.push(button);
                }
            }
            if folding {
                Fold {
                    key,
                    open,
                    before: false,
                    body,
                }
                .append(head, &mut items, layout.limit);
            } else {
                items.push(head);
                items.extend(body);
            }
        }
        if layout.searching {
            if !listing.archived.is_empty() {
                items.push(Item::new(Content::Heading("Архив")));
                layout.add_all(&listing.archived, 0, &mut items, cx);
            }
        } else if listing.archived_count > 0 {
            let animate = listing.archived_count <= layout.limit;
            let toggle = Item::new(Content::Archive {
                count: listing.archived_count,
                animate,
            });
            if archive_moving && animate {
                let mut body = Vec::new();
                layout.add_all(&listing.archived, 0, &mut body, cx);
                Fold {
                    key: ARCHIVE.into(),
                    open: self.show_archived,
                    before: false,
                    body,
                }
                .append(toggle, &mut items, layout.limit);
            } else {
                items.push(toggle);
                if self.show_archived {
                    layout.add_all(&listing.archived, 0, &mut items, cx);
                }
            }
        }
        if layout.searching && items.is_empty() {
            items.push(Item::new(Content::Empty));
        }
        let mut shown = Vec::new();
        let mut slots = Vec::with_capacity(items.len());
        for (slot, item) in items.iter().enumerate() {
            item.cursor_targets(slot, &mut shown);
            slots.push(LayoutSlot {
                id: item.id(),
                moving: item.fold.is_some(),
                focus: None,
            });
        }
        if let Some(renaming) = &self.renaming
            && let Some(&(_, slot)) = shown.iter().find(|(key, _)| key.as_ref() == &renaming.key)
        {
            slots[slot].focus = Some(renaming.input.read(cx).focus_handle(cx));
        }
        self.session_list.set_layout(slots, shown, window);
        let entering = self
            .sessions_arrived
            .is_some_and(|at| at.elapsed() < ENTRANCE);
        let scroll = self.session_list.scroll.clone();
        let overflowing = scroll.max_offset_for_scrollbar().y > px(0.);
        let padding = if overflowing { rems(1.) } else { rems(0.5) };
        let owner = cx.entity_id();
        let gutter_probe = canvas(
            {
                let scroll = scroll.clone();
                move |_, _, cx| {
                    if overflowing != (scroll.max_offset_for_scrollbar().y > px(0.)) {
                        cx.notify(owner);
                    }
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size(px(0.));
        let list = list(
            scroll.clone(),
            cx.processor(move |this, index: usize, _, cx| {
                // Keep the list's wheel hitbox full width. GPUI Fast snaps
                // virtual row origins, so apply the side insets to the row.
                div()
                    .pb_0p5()
                    .pl_2()
                    .pr(padding)
                    .w_full()
                    .children(
                        items
                            .get(index)
                            .map(|item| this.render_panel_item(item, entering, at, cx)),
                    )
                    .into_any_element()
            }),
        )
        .size_full()
        .pb_2();
        let list = self
            .session_list_actions(v_flex().flex_1().min_h_0().relative(), cx)
            .child(list)
            .child(gutter_probe)
            .vertical_scrollbar(&scroll);
        v_flex().size_full().child(list)
    }

    fn render_panel_item(
        &self,
        item: &Item,
        entering: bool,
        at: Instant,
        cx: &Context<Self>,
    ) -> AnyElement {
        let row = match &item.content {
            Content::Heading(label) => heading(label, cx).into_any_element(),
            Content::Project {
                path,
                index,
                open,
                animate,
            } => self
                .render_project(*index, path, !open, *animate, cx)
                .into_any_element(),
            Content::More {
                path,
                index,
                hidden,
                all,
                animate,
            } => self
                .render_more(*index, path, *hidden, *all, *animate, cx)
                .into_any_element(),
            Content::Archive { count, animate } => self
                .render_archive_toggle(*count, *animate, cx)
                .into_any_element(),
            Content::Session {
                key,
                index,
                ordinal,
                look,
                has_children,
            } => {
                let Some(summary) = self
                    .sessions
                    .get(*index)
                    .filter(|summary| &summary.key == key.as_ref())
                    .or_else(|| {
                        self.sessions
                            .iter()
                            .find(|summary| &summary.key == key.as_ref())
                    })
                else {
                    return div().into_any_element();
                };
                let mark = prefs::current(cx).sessions.get(key);
                return self.render_panel_fold(
                    item,
                    motion::stagger_in(
                        div().child(self.render_session_item(
                            *ordinal,
                            summary,
                            mark,
                            *has_children,
                            *look,
                            cx,
                        )),
                        ElementId::Name(format!("session-enter-{key}").into()),
                        *ordinal,
                        entering,
                        cx,
                    )
                    .into_any_element(),
                    entering,
                    at,
                    cx,
                );
            }
            Content::Empty => div()
                .px_2()
                .py_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Ничего не найдено")
                .into_any_element(),
        };
        self.render_panel_fold(item, row, entering, at, cx)
    }

    fn render_panel_fold(
        &self,
        item: &Item,
        row: AnyElement,
        entering: bool,
        at: Instant,
        cx: &Context<Self>,
    ) -> AnyElement {
        let Some(fold) = &item.fold else {
            return row;
        };
        let body = motion::reveal_at(fold.key.clone(), fold.open, at, cx, || {
            v_flex()
                .gap_0p5()
                .when(fold.before, |stack| stack.pb_0p5())
                .when(!fold.before, |stack| stack.pt_0p5())
                .children(
                    fold.body
                        .iter()
                        .map(|item| self.render_panel_item(item, entering, at, cx)),
                )
                .into_any_element()
        });
        if fold.before {
            v_flex().children(body).child(row).into_any_element()
        } else {
            v_flex().child(row).children(body).into_any_element()
        }
    }

    /// A project folder's header: folds its chats and starts a new one there.
    fn render_project(
        &self,
        gi: usize,
        path: &Path,
        collapsed: bool,
        animate: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let path = path.to_path_buf();
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
            .text_color(if collapsed {
                theme.muted_foreground
            } else {
                theme.sidebar_foreground
            })
            .cursor_pointer()
            .hover(|row| row.bg(theme.list_hover))
            .child(
                Icon::new(if collapsed {
                    Lucide::Folder
                } else {
                    Lucide::FolderOpen
                })
                .small()
                .flex_none(),
            )
            .child(div().flex_1().min_w_0().child(faded(folder_name(&path))))
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
                if animate {
                    motion::toggled(fold_key(&path), open, cx);
                }
                cx.notify();
            }))
    }

    fn render_more(
        &self,
        gi: usize,
        path: &Path,
        hidden: usize,
        expanded: bool,
        animate: bool,
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
            .child(if expanded {
                "Свернуть".to_string()
            } else if hidden > 0 {
                format!("Показать больше · {hidden}")
            } else {
                "Показать больше".to_string()
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                let expanded = &mut this.session_list.expanded;
                let all = !expanded.remove(&path);
                if all {
                    expanded.insert(path.clone());
                    this.more_codex_sessions(path.clone(), cx);
                }
                if animate {
                    motion::toggled(more_key(&path), all, cx);
                }
                cx.notify();
            }))
    }

    fn render_archive_toggle(
        &self,
        count: usize,
        animate: bool,
        cx: &Context<Self>,
    ) -> impl IntoElement {
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
                cx,
            ))
            .child(format!("Архив · {count}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.show_archived = !this.show_archived;
                if animate {
                    motion::toggled(ARCHIVE, this.show_archived, cx);
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(index: usize) -> Item {
        Item::new(Content::Session {
            key: Rc::new(ChatKey::Codex(index.to_string())),
            index,
            ordinal: index,
            look: Look {
                selected: false,
                cursor: false,
                depth: 1,
                branch_open: false,
                branch_fits: true,
            },
            has_children: false,
        })
    }

    #[::core::prelude::v1::test]
    fn large_project_extras_keep_individual_slots_and_disappear_on_close() {
        let shape = |open| {
            let mut slots = Vec::new();
            Fold {
                key: "more".into(),
                open,
                before: true,
                body: (0..900).map(chat).collect(),
            }
            .append(Item::new(Content::Heading("More")), &mut slots, 30);
            slots
        };

        let open = shape(true);
        assert_eq!(open.len(), 901);
        assert!(open.iter().all(|slot| slot.fold.is_none()));
        let mut targets = Vec::new();
        for (index, item) in open.iter().enumerate() {
            item.cursor_targets(index, &mut targets);
        }
        assert_eq!(targets.len(), 900);
        assert_eq!(targets[899], (Rc::new(ChatKey::Codex("899".into())), 899));
        assert_eq!(shape(false).len(), 1);
    }

    #[::core::prelude::v1::test]
    fn small_nested_reveal_keeps_navigation_order_and_counts_nested_rows() {
        let mut inner = Vec::new();
        Fold {
            key: "branch".into(),
            open: false,
            before: false,
            body: (2..5).map(chat).collect(),
        }
        .append(chat(1), &mut inner, 4);
        assert_eq!(inner.len(), 1);
        let mut projected = Vec::new();
        Fold {
            key: "project".into(),
            open: true,
            before: false,
            body: std::iter::once(chat(0)).chain(inner).collect(),
        }
        .append(Item::new(Content::Heading("Project")), &mut projected, 4);
        // Two immediate items have five chat rows, because the short inner
        // animation is still alive. Leave the inner block in its own slot.
        assert_eq!(projected.len(), 3);
        let mut targets = Vec::new();
        for (index, item) in projected.iter().enumerate() {
            item.cursor_targets(index, &mut targets);
        }
        let expected: Vec<_> = [1, 2, 2, 2, 2]
            .into_iter()
            .enumerate()
            .map(|(index, slot)| (Rc::new(ChatKey::Codex(index.to_string())), slot))
            .collect();
        assert_eq!(targets, expected);
    }
}
