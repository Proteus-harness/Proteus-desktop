// Адаптировано из Zed: crates/git_ui/src/commit_view.rs, crates/editor/src/split_editor_view.rs
//! File headers and independent horizontal viewports. A long line on one
//! side must never push the other side outside its half of the diff.

use super::{DiffView, HEADER, LINE, lines, stat};
use crate::diff::File;
use crate::ui::fade::caption;
use crate::ui::kit::ButtonLabel as _;
use crate::ui::kit::{self, Hint as _, Pressable as _};
use crate::ui::tip::{Tip as _, TooltipAnchor};
use crate::ui::{motion, prefs};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

impl DiffView {
    pub(super) fn file(&self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let file = self.files[index].clone();
        let open = !self.folded.contains(&file.path);
        let theme = cx.theme();
        let (muted, hover, border) = (theme.muted_foreground, theme.list_hover, theme.border);
        let (name, parent) = file
            .path
            .rsplit_once('/')
            .map_or((file.path.as_str(), ""), |(a, b)| (b, a));
        let (icon, color) = match file.status.as_str() {
            "added" | "untracked" => (Lucide::FilePlus, theme.success),
            "deleted" => (Lucide::FileMinus, theme.danger),
            "renamed" => (Lucide::FileSymlink, theme.warning),
            "conflict" => (Lucide::TriangleAlert, theme.warning),
            _ => (Lucide::FileDiff, theme.muted_foreground),
        };
        let group: SharedString = format!("diff-file-{index}").into();
        let anchor = TooltipAnchor::default();
        let header = h_flex()
            .id(ElementId::NamedInteger("diff-file".into(), index as u64))
            .group(group.clone())
            .tip_anchor(anchor.clone())
            .w_full()
            .flex_none()
            .h(prefs::scaled(HEADER))
            .px_2()
            .gap_2()
            .text_sm()
            .bg(theme.background)
            .border_b_1()
            .border_color(border.opacity(0.6))
            .cursor_pointer()
            .track_focus(&self.file_focus[index])
            .pressable(cx)
            .hover(move |style| style.bg(hover))
            .child(motion::chevron(("chevron", index), open, muted))
            .child(Icon::new(icon).small().flex_none().text_color(color))
            .child(
                div()
                    .min_w_0()
                    .flex_shrink(1.)
                    .child(caption("diff-filename", name.to_owned()).anchor(anchor.clone())),
            )
            .when(!parent.is_empty(), |header| {
                header.child(
                    div().min_w_0().flex_1().text_xs().text_color(muted).child(
                        caption("diff-parent", parent.to_owned())
                            .fade_start()
                            .anchor(anchor.clone()),
                    ),
                )
            })
            .child(div().flex_1())
            .child(stat(file.added, file.removed, cx))
            .child(
                div()
                    .id(("diff-actions", index))
                    .invisible()
                    .group_hover(group, |style| style.visible())
                    .when(
                        window.last_input_was_keyboard()
                            && self.file_focus[index].contains_focused(window, cx),
                        |actions| actions.visible(),
                    )
                    .child(
                        kit::button(("copy-patch", index))
                            .quiet(cx)
                            .xsmall()
                            .icon(Lucide::Copy)
                            .disabled(file.patch.is_empty())
                            .hint("Копировать diff файла", cx)
                            .on_click({
                                let patch = file.patch.clone();
                                move |_, _, cx| {
                                    cx.stop_propagation();
                                    cx.write_to_clipboard(ClipboardItem::new_string(patch.clone()));
                                }
                            }),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(index, !open, cx)));
        let header = header.when_some(file.old_path.as_ref(), |header, old| {
            header.tip_text(format!("{old} → {}", file.path))
        });
        let width = self
            .measured
            .as_ref()
            .map(|(_, _, widths)| widths[index])
            .unwrap_or(px(0.));
        let split = self.split()
            && file.lines.iter().any(|line| {
                matches!(
                    line.kind,
                    crate::diff::Kind::Context
                        | crate::diff::Kind::Added
                        | crate::diff::Kind::Removed
                )
            });
        let syntax = self.syntax[index].clone();
        let horizontal = self.list.is_none().then_some(&self.horizontal[index]);
        let body = motion::reveal(self.key(index, cx), open, cx, || {
            if !split {
                return code(file, syntax, None, width, horizontal.map(|h| &h[0]), cx);
            }
            h_flex()
                .w_full()
                .min_w_0()
                .children([true, false].into_iter().map(|old| {
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .when(old, |column| {
                            column.border_r_1().border_color(border.opacity(0.6))
                        })
                        .child(
                            h_flex()
                                .w_full()
                                .h(prefs::scaled(LINE))
                                .px_3()
                                .text_xs()
                                .text_color(muted)
                                .bg(cx.theme().muted.opacity(0.35))
                                .child(if old { "Было" } else { "Стало" }),
                        )
                        .child(code(
                            file.clone(),
                            syntax.clone(),
                            Some(old),
                            width,
                            horizontal.map(|h| &h[if old { 1 } else { 2 }]),
                            cx,
                        ))
                }))
                .into_any_element()
        });
        v_flex()
            .w_full()
            .flex_none()
            .child(header)
            .children(body)
            .into_any_element()
    }
}

fn code(
    file: Rc<File>,
    syntax: Rc<super::syntax::Syntax>,
    side: Option<bool>,
    width: Pixels,
    horizontal: Option<&ScrollHandle>,
    cx: &App,
) -> AnyElement {
    let id = SharedString::from(format!("diff-code-{}-{side:?}", file.path));
    let code = div()
        .w_full()
        .min_w_0()
        .font_family(cx.theme().mono_font_family.clone())
        .text_xs()
        .child(lines::Lines::new(file, syntax, side, width, cx));
    if let Some(horizontal) = horizontal {
        // Unlike overflow_x_scrollbar, this does not remap vertical wheel
        // input onto X: an embedded diff lets it reach the outer chat.
        div()
            .relative()
            .w_full()
            .min_w_0()
            .overflow_x_hidden()
            .child(
                code.id(id)
                    .overflow_x_scroll()
                    .lock_scroll_axis()
                    .track_scroll(horizontal),
            )
            .horizontal_scrollbar(horizontal)
            .into_any_element()
    } else {
        code.overflow_x_scrollbar().id(id).into_any_element()
    }
}
