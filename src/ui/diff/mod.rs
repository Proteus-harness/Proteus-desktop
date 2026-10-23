// Адаптировано из Zed: crates/git_ui/src/commit_view.rs, crates/editor/src/split.rs, crates/editor/src/element.rs
//! Shared read-only diff for documents and tool calls. File cards share the
//! disclosure motion of trees and settings; `lines` paints only the visible
//! source rows, even while the entire file is opening or closing.

use crate::ui::kit::ButtonLabel as _;
mod file;
mod lines;
mod row;
mod syntax;

use std::collections::HashSet;
use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::diff::{self, File, FileInfo};
use crate::ui::fade::faded;
use crate::ui::kit::{self, Hint as _};
use crate::ui::{motion, prefs};
use gpui_kit::component::scroll::ScrollableElement as _;

const LINE: f32 = 20.;
const HEADER: f32 = 32.;
/// Zed's minimum_split_diff_width, measured in monospace advances.
const SPLIT_EMS: f32 = 100.;

pub(in crate::ui) struct DiffView {
    patch: String,
    info: Vec<FileInfo>,
    files: Vec<Rc<File>>,
    syntax: Vec<Rc<syntax::Syntax>>,
    syntax_theme: std::sync::Arc<gpui_kit::component::highlighter::HighlightTheme>,
    folded: HashSet<String>,
    list: ListState,
    /// The user's choice survives an automatic narrow-width fallback.
    split: bool,
    narrow: bool,
    compact: bool,
    focus: FocusHandle,
    file_focus: Vec<FocusHandle>,
    /// Widths are shaped once per font/size, never on every animation frame.
    measured: Option<(Font, Pixels, Vec<Pixels>)>,
}

impl DiffView {
    pub fn new(patch: String, info: Vec<FileInfo>, cx: &mut Context<Self>) -> Self {
        let files: Vec<_> = diff::parse(&patch, &info)
            .into_iter()
            .map(Rc::new)
            .collect();
        let list = ListState::new(files.len(), ListAlignment::Top, px(160.));
        let file_focus = files.iter().map(|_| cx.focus_handle()).collect();
        let syntax_theme = cx.theme().highlight_theme.clone();
        let syntax = files
            .iter()
            .map(|file| Rc::new(syntax::Syntax::new(file, &syntax_theme)))
            .collect();
        Self {
            patch,
            info,
            files,
            syntax,
            syntax_theme,
            folded: HashSet::new(),
            list,
            split: true,
            narrow: true,
            compact: false,
            focus: cx.focus_handle(),
            file_focus,
            measured: None,
        }
    }

    pub fn set_patch(&mut self, patch: &str, info: &[FileInfo], cx: &mut Context<Self>) {
        if self.patch == patch && self.info == info {
            return;
        }
        self.patch = patch.into();
        self.info = info.to_vec();
        let anchor = self.list.logical_scroll_top();
        let anchor_path = self.files.get(anchor.item_ix).map(|file| file.path.clone());
        let files: Vec<_> = diff::parse(patch, info).into_iter().map(Rc::new).collect();
        let same_order = self.files.len() == files.len()
            && self.files.iter().zip(&files).all(|(a, b)| a.path == b.path);
        if same_order {
            self.list.remeasure_items(0..files.len());
        } else {
            self.list.splice(0..self.list.item_count(), files.len());
            if let Some(index) = files
                .iter()
                .position(|file| Some(&file.path) == anchor_path.as_ref())
            {
                self.list.scroll_to(ListOffset {
                    item_ix: index,
                    offset_in_item: anchor.offset_in_item,
                });
            }
        }
        self.file_focus = files
            .iter()
            .map(|file| {
                self.files
                    .iter()
                    .position(|old| old.path == file.path)
                    .map(|index| self.file_focus[index].clone())
                    .unwrap_or_else(|| cx.focus_handle())
            })
            .collect();
        self.syntax_theme = cx.theme().highlight_theme.clone();
        self.syntax = files
            .iter()
            .map(|file| Rc::new(syntax::Syntax::new(file, &self.syntax_theme)))
            .collect();
        self.files = files;
        self.folded
            .retain(|path| self.files.iter().any(|file| &file.path == path));
        self.measured = None;
        cx.notify();
    }

    /// An embedded chat diff grows to a modest height, then scrolls inside.
    pub fn embedded_height(&self) -> f32 {
        let rows: usize = self.files.iter().map(|file| file.unified.len()).sum();
        (HEADER * (self.files.len() + 1) as f32 + LINE * rows as f32).clamp(100., 360.)
    }

    pub fn focus(&self) -> FocusHandle {
        self.focus.clone()
    }

    fn key(&self, index: usize, cx: &Context<Self>) -> SharedString {
        format!("diff-{}-{}", cx.entity_id(), self.files[index].path).into()
    }

    fn toggle(&mut self, index: usize, open: bool, cx: &mut Context<Self>) {
        let Some(file) = self.files.get(index) else {
            return;
        };
        if open {
            self.folded.remove(&file.path);
        } else {
            self.folded.insert(file.path.clone());
        }
        motion::toggled(self.key(index, cx), open, cx);
        // Do not reveal/scroll the whole file: the clicked header stays put.
        self.list.remeasure_items(index..index + 1);
        cx.notify();
    }

    fn fold_all(&mut self, cx: &mut Context<Self>) {
        let open = !self.folded.is_empty();
        for index in 0..self.files.len() {
            if self.folded.contains(&self.files[index].path) == open {
                self.toggle(index, open, cx);
            }
        }
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let (added, removed) = self
            .files
            .iter()
            .fold((0, 0), |(a, r), f| (a + f.added, r + f.removed));
        let muted = cx.theme().muted_foreground;
        h_flex()
            .flex_none()
            .w_full()
            .h(prefs::scaled(HEADER))
            .px_2()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_xs()
            .child(
                div()
                    .min_w_0()
                    .flex_shrink(1.)
                    .text_color(muted)
                    .child(faded(format!("Файлов: {}", self.files.len()))),
            )
            .child(stat(added, removed, cx))
            .child(div().flex_1())
            .children(
                [
                    (false, Lucide::Rows2, "Unified", "Изменения в одной колонке"),
                    (
                        true,
                        Lucide::Columns2,
                        "Split",
                        "Было и стало рядом; на узкой ширине — одна колонка",
                    ),
                ]
                .into_iter()
                .map(|(split, icon, label, hint)| {
                    kit::button(if split { "split-diff" } else { "unified-diff" })
                        .ghost()
                        .xsmall()
                        .icon(icon)
                        .when(!self.compact, |button| button.faded_label(label))
                        .selected(self.split == split)
                        .hint(hint, cx)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.split = split;
                            this.list.remeasure_items(0..this.files.len());
                            cx.notify();
                        }))
                }),
            )
            .child(div().h(prefs::scaled(16.)).w(px(1.)).bg(cx.theme().border))
            .child(
                kit::button("fold-diffs")
                    .ghost()
                    .xsmall()
                    .icon(if self.folded.is_empty() {
                        Lucide::FoldVertical
                    } else {
                        Lucide::UnfoldVertical
                    })
                    .hint(
                        if self.folded.is_empty() {
                            "Свернуть все файлы"
                        } else {
                            "Развернуть все файлы"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.fold_all(cx))),
            )
            .child(
                kit::button("copy-diffs")
                    .ghost()
                    .xsmall()
                    .icon(Lucide::Copy)
                    .disabled(self.patch.is_empty())
                    .hint("Копировать весь diff", cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(this.patch.clone()));
                    })),
            )
    }
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        motion::drive(window, cx);
        if !std::sync::Arc::ptr_eq(&self.syntax_theme, &cx.theme().highlight_theme) {
            self.syntax_theme = cx.theme().highlight_theme.clone();
            self.syntax = self
                .files
                .iter()
                .map(|file| Rc::new(syntax::Syntax::new(file, &self.syntax_theme)))
                .collect();
        }
        let font = Font {
            family: cx.theme().mono_font_family.clone(),
            ..Font::default()
        };
        let font_size = prefs::scaled(12.).to_pixels(window.rem_size());
        if self
            .measured
            .as_ref()
            .is_none_or(|(f, s, _)| f != &font || *s != font_size)
        {
            let run = |len| TextRun {
                len,
                font: font.clone(),
                color: cx.theme().foreground.into(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            let widths = self
                .files
                .iter()
                .map(|file| {
                    file.lines
                        .iter()
                        .map(|line| {
                            let text: SharedString = line.text.replace('\t', "    ").into();
                            window
                                .text_system()
                                .shape_line(text.clone(), font_size, &[run(text.len())], None)
                                .width()
                        })
                        .fold(px(0.), |a, b| a.max(b))
                })
                .collect();
            self.measured = Some((font.clone(), font_size, widths));
            self.list.remeasure_items(0..self.files.len());
        }
        // Like Zed, use actual monospace glyph advances, not a window-size guess.
        let font_id = window.text_system().resolve_font(&font);
        let minimum = window
            .text_system()
            .em_advance(font_id, font_size)
            .unwrap_or(font_size)
            * SPLIT_EMS;
        let view = cx.weak_entity();
        let compact_width = prefs::scaled(480.).to_pixels(window.rem_size());
        let probe = canvas(
            move |bounds, _, cx| {
                let narrow = bounds.size.width < minimum;
                let compact = bounds.size.width < compact_width;
                cx.defer(move |cx| {
                    let _ = view.update(cx, |this, cx| {
                        if this.narrow != narrow || this.compact != compact {
                            this.narrow = narrow;
                            this.compact = compact;
                            this.list.remeasure_items(0..this.files.len());
                            cx.notify();
                        }
                    });
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        // A disclosure changes its item's height on every frame; preserve the
        // absolute list anchor, including the final settled frame.
        self.list.remeasure_items(0..self.files.len());
        let content = if self.files.is_empty() {
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Изменений нет")
                .into_any_element()
        } else {
            list(
                self.list.clone(),
                cx.processor(|this, index, window, cx| this.file(index, window, cx)),
            )
            .flex_1()
            .min_h_0()
            .w_full()
            .into_any_element()
        };
        v_flex()
            .relative()
            .size_full()
            .min_w_0()
            .overflow_hidden()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "pageup" => this
                        .list
                        .scroll_by(-this.list.viewport_bounds().size.height),
                    "pagedown" => this.list.scroll_by(this.list.viewport_bounds().size.height),
                    "home" => this.list.scroll_to(ListOffset::default()),
                    "end" => this.list.scroll_to_end(),
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(probe)
            .child(self.toolbar(cx))
            .child(
                v_flex()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(content)
                    .vertical_scrollbar(&self.list),
            )
    }
}

/// The common +N −M label used by file cards, trees and tool headlines.
pub(in crate::ui) fn stat(added: usize, removed: usize, cx: &App) -> Div {
    h_flex()
        .flex_none()
        .gap_1()
        .text_xs()
        .child(
            div()
                .text_color(cx.theme().success)
                .child(format!("+{added}")),
        )
        .child(
            div()
                .text_color(cx.theme().danger)
                .child(format!("−{removed}")),
        )
}
