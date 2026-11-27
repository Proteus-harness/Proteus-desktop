// Адаптировано из Zed: crates/workspace/src/pane.rs (update_active_tab)
use crate::ui::kit::{ButtonLabel as _, WorkspaceSurface as _};
use std::time::Instant;

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::motion::Frame;
use super::{drop, split};
use crate::tabs::{self, Axis, Node, Pane, PaneId, Size, TabLayout};
use crate::ui::kit;
use crate::ui::workspace::Workspace;
use crate::ui::{motion, prefs};

impl Workspace {
    /// The tree of panes with their bars, contents and dividers.
    pub(in crate::ui) fn render_tabs(
        &mut self,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut layout = self.tab_layout(cx);
        let compact = compact_chat_and_files(&layout, width);
        let compact_changed = compact != self.compact_split;
        self.compact_split = compact;
        if let Some((chat, files)) = compact
            && compact_changed
            && layout.focused == files
        {
            let editor_focused = layout
                .active(files)
                .and_then(|id| self.documents.get(id))
                .is_some_and(|editor| editor.focus.contains_focused(window, cx));
            layout.focus(chat);
            self.layout.focus(chat);
            if editor_focused {
                let workspace = cx.weak_entity();
                window.defer(cx, move |window, cx| {
                    let _ = workspace.update(cx, |this, cx| {
                        if this.compact_split == compact && this.layout.focused == chat {
                            this.focus_active(window, cx);
                        }
                    });
                });
            }
        }
        if !cx.has_active_drag() {
            self.tab_drag = false;
            self.tab_drop = None;
        }
        self.sync_tab_strips(&layout);
        let now = Instant::now();
        let running = self.tab_motion.tick(now);
        let frames = self.tab_area.get().and_then(|area| {
            // The layout is measured at 100% scale.
            let size = (width, prefs::unscaled(area.height, window));
            let target = layout.rects_spaced(size.0, size.1, split::DIVIDER);
            let panes = self.tab_motion.panes();
            panes.retarget(
                size,
                target,
                self.split_drag.is_some() || compact_changed || compact.is_some(),
                now,
                crate::ui::motion::moves(cx),
            );
            panes.sample(now)
        });
        if running || frames.is_some() {
            window.request_animation_frame();
        }
        let area = self.tab_area.clone();
        let probe = canvas(
            move |bounds, _, _| area.set(Some(bounds.size)),
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let content = match (compact, frames) {
            (Some(_), _) => self
                .render_pane(
                    layout
                        .pane(layout.focused)
                        .expect("focused pane is in the layout"),
                    &layout,
                    window,
                    cx,
                )
                .into_any_element(),
            (None, Some(frames)) => self.render_moving_panes(&frames, &layout, window, cx),
            (None, None) => self.render_node(&layout.root, Vec::new(), &layout, window, cx),
        };
        div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(probe)
            .child(content)
    }

    /// Panes placed where they are while they glide to a new arrangement.
    /// Each keeps its final size and is clipped by its moving frame, so
    /// contents do not reflow on every frame; layout seams stay transparent.
    fn render_moving_panes(
        &mut self,
        frames: &[Frame],
        layout: &TabLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let line = cx.theme().transparent;
        let half = split::DIVIDER / 2.;
        let mut area = div().relative().size_full();
        for frame in frames {
            let Some(pane) = layout.pane(frame.id) else {
                continue;
            };
            let (rect, target) = (frame.rect, frame.target);
            let content = self.render_pane(pane, layout, window, cx);
            area = area.child(
                div()
                    .absolute()
                    .left(prefs::scaled(rect.x))
                    .top(prefs::scaled(rect.y))
                    .w(prefs::scaled(rect.w))
                    .h(prefs::scaled(rect.h))
                    .overflow_hidden()
                    .opacity(frame.opacity)
                    .child(
                        div()
                            .w(prefs::scaled(target.w))
                            .h(prefs::scaled(target.h))
                            .child(content),
                    ),
            );
            if rect.x > half {
                area = area.child(
                    div()
                        .absolute()
                        .left(prefs::scaled(rect.x - half))
                        .top(prefs::scaled(rect.y))
                        .w(px(1.))
                        .h(prefs::scaled(rect.h))
                        .bg(line),
                );
            }
            if rect.y > half {
                area = area.child(
                    div()
                        .absolute()
                        .left(prefs::scaled(rect.x))
                        .top(prefs::scaled(rect.y - half))
                        .w(prefs::scaled(rect.w))
                        .h(px(1.))
                        .bg(line),
                );
            }
        }
        area.into_any_element()
    }

    /// Scrolls a newly active tab into view in its strip.
    fn sync_tab_strips(&mut self, layout: &TabLayout) {
        let panes = layout.panes();
        self.tab_strips
            .retain(|id, _| panes.iter().any(|pane| pane.id == *id));
        let now = Instant::now();
        for pane in panes {
            if self.tab_drag {
                continue;
            }
            let (strip, shown) = self.tab_strips.entry(pane.id).or_default();
            let active = pane.active.as_ref().and_then(|active| {
                pane.tabs
                    .iter()
                    .position(|id| id == active)
                    .map(|ix| (ix, active))
            });
            // Pinning or reordering can move the same active tab out of
            // view. Its position matters as well as its id.
            let leaving = self.tab_motion.leaving(pane.id, now);
            let target = active.map(|(mut ix, id)| {
                // Shrinking closed tabs still occupy children of the strip.
                // Use the rendered index until those children disappear.
                for (closed, _) in &leaving {
                    if closed.index <= ix {
                        ix += 1;
                    }
                }
                (ix, id)
            });
            let key = target.map(|(ix, id)| format!("{ix}:{id}"));
            let resizing = !leaving.is_empty()
                || active.is_some_and(|(_, id)| self.tab_motion.entering(id, now).is_some());
            if *shown != key || resizing {
                if let Some((ix, _)) = target {
                    strip.scroll_to_item(ix);
                }
                *shown = key;
            }
        }
    }

    fn render_node(
        &mut self,
        node: &Node,
        path: Vec<usize>,
        layout: &TabLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let split = match node {
            Node::Pane(pane) => {
                return self
                    .render_pane(pane, layout, window, cx)
                    .into_any_element();
            }
            Node::Split(split) => split,
        };
        let axis = split.axis;
        let container = match axis {
            Axis::Row => h_flex(),
            Axis::Column => v_flex(),
        };
        let mut container = split::track_resize(
            container.size_full().min_w_0().min_h_0(),
            path.clone(),
            axis,
            cx,
        );
        for (ix, child) in split.children.iter().enumerate() {
            if ix > 0 {
                container = container.child(split::divider(path.clone(), ix - 1, axis, cx));
            }
            let mut child_path = path.clone();
            child_path.push(ix);
            let content = self.render_node(&child.node, child_path, layout, window, cx);
            container = container.child(
                div()
                    .map(|cell| match child.size {
                        Size::Weight(weight) => cell.flex_basis(relative(0.)).flex_grow(weight),
                        Size::Fixed(len) => cell.flex_basis(prefs::scaled(len)).flex_grow(0.),
                    })
                    .flex_shrink(1.)
                    .min_w_0()
                    .min_h_0()
                    .map(|cell| match axis {
                        Axis::Row => cell.h_full(),
                        Axis::Column => cell.w_full(),
                    })
                    .child(content),
            );
        }
        container.into_any_element()
    }

    fn render_pane(
        &mut self,
        pane: &Pane,
        layout: &TabLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = pane.id;
        let content = match pane.active.as_deref() {
            Some(tab) if tabs::is_chat(tab) => match self
                .chats
                .get(tab)
                .map(|chat| &chat.view)
                .or_else(|| self.opening_views.get(tab))
            {
                // Ordinary retained views splice dirty descendants without
                // rendering the whole chat. An explicit cached-style wrapper
                // instead rebuilds this parent on each caption animation tick.
                Some(view) => div()
                    .size_full()
                    .child(AnyView::from(view.clone()))
                    .into_any_element(),
                None => {
                    if let Some(key) = tabs::chat_key(tab) {
                        self.ensure_chat(key, window, cx);
                    }
                    super::super::state::loading(
                        format!("chat-loading-{tab}"),
                        "Открытие чата…",
                        cx,
                    )
                }
            },
            Some(tab) => match self.ensure_document(tab, window, cx) {
                Some(document) => motion::rise_in_if(
                    div().size_full().child(document.view),
                    ElementId::Name(format!("tab-document-{tab}").into()),
                    self.tab_motion.opened(tab, Instant::now()),
                 cx)
                .into_any_element(),
                None => div()
                    .p_4()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Это расширение требует чат Proteus Core. Откройте такой чат, чтобы использовать эту вкладку.")
                    .into_any_element(),
            },
            None => self.render_empty_pane(id, cx).into_any_element(),
        };
        let body = div()
            .relative()
            .flex_1()
            .min_h_0()
            .bg(cx.theme().background)
            .child(content)
            .workspace_surface(cx.theme().radius_lg, cx.theme().sidebar);
        v_flex()
            .relative()
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| this.focus_pane(id, cx)),
            )
            .child(self.render_tab_bar(pane, layout, cx))
            .child(drop::accept_tabs(body, id, cx).children(self.drop_highlight(id, cx)))
    }

    /// An empty pane explains how to fill it.
    fn render_empty_pane(&self, pane: PaneId, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .px_6()
            .child(
                div()
                    .text_sm()
                    .text_center()
                    .text_color(theme.muted_foreground)
                    .child(
                        "Перетащите сюда вкладку или откройте сессию или файл на панелях \
                         «Сессии» и «Файлы».",
                    ),
            )
            .children(self.notice.clone().map(|notice| {
                div()
                    .text_xs()
                    .text_center()
                    .text_color(crate::ui::theme::danger_text(&theme))
                    .child(notice)
            }))
            .when(self.chats.is_empty(), |column| {
                column.child(
                    kit::button(("empty-pane-new-session", pane.0 as usize))
                        .quiet(cx)
                        .small()
                        .icon(IconName::Plus)
                        .faded_label("Новая сессия")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.new_session(Some(pane), window, cx)
                        })),
                )
            })
    }
}

/// When the automatic chat/file split falls below the divider's minimum,
/// show one full-width pane. Keep both panes and their shares in the tab tree
/// so reopening a file and growing the window can use the same editor.
fn compact_chat_and_files(layout: &TabLayout, width: f32) -> Option<(PaneId, PaneId)> {
    let Node::Split(root) = &layout.root else {
        return None;
    };
    let [first, second] = root.children.as_slice() else {
        return None;
    };
    let (Node::Pane(first), Node::Pane(second)) = (&first.node, &second.node) else {
        return None;
    };
    if root.axis != Axis::Row {
        return None;
    }
    let is_chat = |pane: &Pane| pane.active.as_deref().is_some_and(tabs::is_chat);
    let is_files = |pane: &Pane| {
        !pane.tabs.is_empty()
            && pane.tabs.iter().all(|tab| {
                tabs::split_document(tab).is_some_and(|(extension, _)| extension == "files")
            })
    };
    let pair = if is_chat(first) && is_files(second) {
        (first.id, second.id)
    } else if is_chat(second) && is_files(first) {
        (second.id, first.id)
    } else {
        return None;
    };
    layout
        .rects_spaced(width, 0., split::DIVIDER)
        .iter()
        .any(|(_, rect)| rect.w < split::MIN_WIDTH - 0.5)
        .then_some(pair)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn a_cramped_horizontal_chat_and_file_pair_compacts_without_changing_the_tree() {
        let mut layout = TabLayout::default();
        layout.open("chat:/fixture/chat", |_| true);
        layout.open("files:notes.md", |_| true);
        let chat = layout.pane_of("chat:/fixture/chat").unwrap();
        let file = layout.pane_of("files:notes.md").unwrap();
        let saved = layout.clone();
        let minimum = 2. * split::MIN_WIDTH + split::DIVIDER;

        assert_eq!(compact_chat_and_files(&layout, 960.), None);
        assert_eq!(compact_chat_and_files(&layout, minimum), None);
        assert_eq!(
            compact_chat_and_files(&layout, minimum - 80.),
            Some((chat, file))
        );
        assert_eq!(compact_chat_and_files(&layout, 960.), None);
        assert_eq!(layout, saved);
    }

    #[::core::prelude::v1::test]
    fn compact_pair_checks_actual_split_shares_and_the_type_of_the_pane() {
        let mut layout = TabLayout::default();
        layout.open("chat:/fixture/chat", |_| true);
        layout.open("files:notes.md", |_| true);
        let chat = layout.pane_of("chat:/fixture/chat").unwrap();
        let file = layout.pane_of("files:notes.md").unwrap();
        layout.set_sizes(&[], &[Size::Weight(3.), Size::Weight(1.)]);
        assert_eq!(compact_chat_and_files(&layout, 850.), Some((chat, file)));
        assert_eq!(compact_chat_and_files(&layout, 1000.), None);

        layout.open_in("terminal:example", file, |_| false);
        assert_eq!(compact_chat_and_files(&layout, 850.), None);
    }
}
