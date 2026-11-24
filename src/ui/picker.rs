// Адаптировано из Zed: crates/picker/src/picker.rs (query, selection and reveal-selected scrolling)
// and crates/agent_ui/src/language_model_selector.rs (group headings).
//! A searchable, virtualized single-choice control. Callers supply labels and
//! values and may group adjacent choices; headings never become choices.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

use super::kit::{ButtonLabel as _, Hint as _};
use super::{fade, prefs, selection};

const ROW: f32 = 28.;
const VISIBLE: usize = 8;

#[derive(Clone, PartialEq)]
pub(super) struct Choice<T> {
    pub label: SharedString,
    pub value: T,
    pub selected: bool,
    pub group: Option<SharedString>,
}

#[derive(Debug, PartialEq, Eq)]
struct MatchedChoice {
    option: usize,
    row: usize,
}

#[derive(Debug, PartialEq, Eq)]
enum Row {
    Heading(SharedString),
    Choice(usize),
}

type Pick<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;
type Back = Rc<dyn Fn(&mut Window, &mut App)>;
type RenderLabel<T> = Rc<dyn Fn(&Choice<T>, &mut Window, &mut App) -> AnyElement>;

pub(super) struct Picker<T: Clone + PartialEq + 'static> {
    search: Entity<InputState>,
    searchable: bool,
    title: Option<SharedString>,
    back: Option<Back>,
    list_focus: FocusHandle,
    options: Vec<Choice<T>>,
    matches: Vec<MatchedChoice>,
    rows: Vec<Row>,
    cursor: usize,
    empty: SharedString,
    placeholder: SharedString,
    search_label: Option<SharedString>,
    scroll: UniformListScrollHandle,
    pick: Pick<T>,
    render_label: Option<RenderLabel<T>>,
    height_limit: Option<fn(&Window) -> Pixels>,
    _search: Subscription,
}

impl<T: Clone + PartialEq + 'static> Picker<T> {
    pub fn new(
        options: Vec<Choice<T>>,
        placeholder: SharedString,
        empty: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
        pick: impl Fn(T, &mut Window, &mut App) + 'static,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder.clone()));
        let subscription = cx.subscribe(&search, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.filter(None, cx);
            }
        });
        let mut picker = Self {
            search,
            searchable: true,
            title: None,
            back: None,
            list_focus: cx.focus_handle(),
            options,
            matches: Vec::new(),
            rows: Vec::new(),
            cursor: 0,
            empty,
            placeholder,
            search_label: None,
            scroll: UniformListScrollHandle::new(),
            pick: Rc::new(pick),
            render_label: None,
            height_limit: None,
            _search: subscription,
        };
        picker.filter(None, cx);
        picker
    }

    pub fn set_options(
        &mut self,
        options: Vec<Choice<T>>,
        placeholder: SharedString,
        empty: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.placeholder != placeholder {
            self.placeholder = placeholder.clone();
            self.search.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        if self.options != options || self.empty != empty {
            let cursor = self.current().map(|choice| choice.value.clone());
            self.options = options;
            self.empty = empty;
            self.filter(cursor, cx);
        }
    }

    pub fn set_pick(&mut self, pick: impl Fn(T, &mut Window, &mut App) + 'static) {
        self.pick = Rc::new(pick);
    }

    /// Name search for assistive technology without adding visible copy.
    pub fn search_label(mut self, label: impl Into<SharedString>) -> Self {
        self.search_label = Some(label.into());
        self
    }

    /// Customize only the caption; row geometry, selection, focus and keys
    /// remain shared with every other picker.
    pub fn render_label(
        mut self,
        render: impl Fn(&Choice<T>, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.render_label = Some(Rc::new(render));
        self
    }

    /// Bound the list and its header together, leaving the list itself scrollable.
    pub fn height_limit(mut self, limit: fn(&Window) -> Pixels) -> Self {
        self.height_limit = Some(limit);
        self
    }

    pub fn set_header(
        &mut self,
        title: Option<SharedString>,
        back: Option<Back>,
        cx: &mut Context<Self>,
    ) {
        let changed = self.title != title || self.back.is_some() != back.is_some();
        self.title = title;
        self.back = back;
        if changed {
            cx.notify();
        }
    }

    /// The same rows, cursor and keyboard handling serve compact selectors.
    pub fn set_searchable(
        &mut self,
        searchable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.searchable == searchable {
            return;
        }
        let focused = self.focus(cx).contains_focused(window, cx);
        self.searchable = searchable;
        if !searchable {
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.filter(None, cx);
        if focused {
            self.focus(cx).focus(window, cx);
        }
    }

    pub fn focus(&self, _: &App) -> FocusHandle {
        self.list_focus.clone()
    }

    pub fn search_focus(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }

    pub fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        self.list_focus.contains_focused(window, cx)
    }

    fn current(&self) -> Option<&Choice<T>> {
        self.matches
            .get(self.cursor)
            .map(|item| &self.options[item.option])
    }

    fn reveal_cursor(&self) {
        if let Some(item) = self.matches.get(self.cursor) {
            // Start at the first heading when the first choice is selected.
            let row = if self.cursor == 0 { 0 } else { item.row };
            self.scroll.scroll_to_item(row, ScrollStrategy::Nearest);
        }
    }

    fn filter(&mut self, keep: Option<T>, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_lowercase();
        (self.matches, self.rows) = matching(&self.options, &query);
        self.cursor = keep
            .as_ref()
            .and_then(|value| {
                self.matches
                    .iter()
                    .position(|item| &self.options[item.option].value == value)
            })
            .or_else(|| {
                query
                    .is_empty()
                    .then(|| {
                        self.matches
                            .iter()
                            .position(|item| self.options[item.option].selected)
                    })
                    .flatten()
            })
            .unwrap_or(0);
        self.reveal_cursor();
        cx.notify();
    }

    fn keys(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.searchable
            && event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
            && !event.keystroke.modifiers.shift
            && event.keystroke.key == "f"
        {
            super::search::focus(&self.search, window, cx);
            cx.notify();
            cx.stop_propagation();
            return;
        }
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" | "down" | "pageup" | "pagedown" if !self.matches.is_empty() => {
                let count = self.matches.len();
                let row = self.matches[self.cursor].row;
                self.cursor = match event.keystroke.key.as_str() {
                    "up" => (self.cursor + count - 1) % count,
                    "down" => (self.cursor + 1) % count,
                    "pageup" => self
                        .matches
                        .partition_point(|item| item.row < row.saturating_sub(VISIBLE)),
                    _ => {
                        self.matches
                            .partition_point(|item| item.row <= row + VISIBLE)
                            - 1
                    }
                };
                self.reveal_cursor();
                cx.notify();
            }
            // Search and Back are real header buttons: their focused Enter
            // must reach the native press/release handler, not commit a row.
            "enter"
                if self.list_focus.is_focused(window)
                    || self.search_focus(cx).is_focused(window) =>
            {
                if let Some(choice) = self.current() {
                    (self.pick)(choice.value.clone(), window, cx);
                }
            }
            // Esc first clears the local filter. The next Esc can dismiss
            // the containing popup, without selecting or changing anything.
            "escape" if self.search_focus(cx).is_focused(window) => return,
            _ => return,
        }
        cx.stop_propagation();
    }
}

fn matching<T>(options: &[Choice<T>], query: &str) -> (Vec<MatchedChoice>, Vec<Row>) {
    let terms: Vec<_> = query.split_whitespace().collect();
    let mut matches = Vec::new();
    let mut rows = Vec::new();
    let mut previous_group = None;
    for (index, option) in options.iter().enumerate() {
        let group = option
            .group
            .as_ref()
            .filter(|group| !group.trim().is_empty());
        if !terms.is_empty() {
            let label = option.label.to_lowercase();
            let heading = group.map(|group| group.to_lowercase());
            if !terms.iter().all(|term| {
                label.contains(term) || heading.as_ref().is_some_and(|group| group.contains(term))
            }) {
                continue;
            }
        }
        if group != previous_group
            && let Some(group) = group
        {
            rows.push(Row::Heading(group.clone()));
        }
        previous_group = group;
        let row = rows.len();
        rows.push(Row::Choice(matches.len()));
        matches.push(MatchedChoice { option: index, row });
    }
    (matches, rows)
}

impl<T: Clone + PartialEq + 'static> Render for Picker<T> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.rows.len();
        let focused = self.contains_focused(window, cx);
        let rows = uniform_list(
            "picker-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, window, cx| {
                range
                    .map(|row| {
                        let index = match &this.rows[row] {
                            Row::Heading(title) => {
                                return h_flex()
                                    .id(("picker-heading", row))
                                    .role(Role::Heading)
                                    .aria_label(title.clone())
                                    .w_full()
                                    .min_w_0()
                                    .h(prefs::scaled(ROW))
                                    .px_2()
                                    .items_center()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .when(row > 0, |header| {
                                        header.border_t_1().border_color(cx.theme().border)
                                    })
                                    .child(fade::caption("caption", title.clone()))
                                    .into_any_element();
                            }
                            Row::Choice(index) => *index,
                        };
                        let option = &this.options[this.matches[index].option];
                        let cursor = index == this.cursor;
                        let value = option.value.clone();
                        let label = match &this.render_label {
                            Some(render) => render(option, window, cx),
                            None => {
                                fade::caption("caption", option.label.clone()).into_any_element()
                            }
                        };
                        h_flex()
                            .id(("picker-row", row))
                            .role(Role::ListBoxOption)
                            .aria_label(option.label.clone())
                            .aria_selected(option.selected)
                            .relative()
                            .w_full()
                            .min_w_0()
                            .group("picker-selection")
                            .h(prefs::scaled(ROW))
                            .px_2()
                            .gap_2()
                            .items_center()
                            .text_sm()
                            .cursor_pointer()
                            .child(selection::background(
                                option.selected,
                                cursor && focused,
                                "picker-selection",
                                true,
                                cx,
                            ))
                            .child(div().flex_1().min_w_0().child(label))
                            .when(cursor && focused && super::kit::keyboard(), |row| {
                                row.child(selection::focus(cx))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.cursor = index;
                                cx.notify();
                                (this.pick)(value.clone(), window, cx);
                            }))
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(prefs::scaled(ROW * count.min(VISIBLE) as f32));
        v_flex()
            .id("picker")
            .track_focus(&self.list_focus)
            .tab_stop(true)
            .when_some(self.height_limit, |body, limit| body.max_h(limit(window)))
            .w_full()
            .min_w_0()
            .gap_1()
            .capture_key_down(cx.listener(Self::keys))
            .when(self.searchable || self.title.is_some(), |body| {
                let mut header = h_flex().w_full().min_w_0().items_center().gap_1();
                if self.back.is_some() {
                    header = header.child(
                        super::kit::button("picker-back")
                            .small()
                            .quiet(cx)
                            .icon(IconName::ChevronLeft)
                            .hint("Назад", cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(back) = this.back.clone() {
                                    back(window, cx);
                                }
                            })),
                    );
                }
                if self.searchable {
                    header = header.child(
                        super::search::Search::new(
                            "picker-search",
                            self.search.clone(),
                            self.list_focus.clone(),
                            self.search_label
                                .clone()
                                .unwrap_or_else(|| self.placeholder.clone()),
                        )
                        .title(
                            self.title
                                .clone()
                                .unwrap_or_else(|| self.placeholder.clone()),
                        ),
                    );
                } else if let Some(title) = self.title.clone() {
                    header = header.child(
                        div()
                            .px_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(fade::faded(title)),
                    );
                }
                body.child(header)
            })
            .when(count > 0, |body| body.child(rows))
            .when(count == 0, |body| {
                body.child(
                    div()
                        .px_2()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.empty.clone()),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn query_filters_unicode_words_without_changing_option_identity() {
        let options: Vec<_> = ["OpenAI / Обычная", "Local / Обычная", "OpenAI / Быстрая"]
            .into_iter()
            .enumerate()
            .map(|(value, label)| Choice {
                label: label.into(),
                value,
                selected: value == 1,
                group: None,
            })
            .collect();
        let indices = |query| {
            matching(&options, query)
                .0
                .into_iter()
                .map(|choice| choice.option)
                .collect::<Vec<_>>()
        };
        assert_eq!(indices("openai обыч"), vec![0]);
        assert_eq!(indices("обычная"), vec![0, 1]);
        assert!(indices("none").is_empty());
        assert_eq!(indices(""), vec![0, 1, 2]);
    }

    #[::core::prelude::v1::test]
    fn provider_queries_keep_matching_sections_and_only_options_are_selectable() {
        let options = [
            ("OpenAI", "Astra"),
            ("OpenAI", "Sol"),
            ("OpenCode Go", "DeepSeek Flash"),
            ("OpenCode Go", "Astra"),
        ]
        .map(|(group, label)| Choice {
            label: label.into(),
            value: (group, label),
            selected: false,
            group: Some(group.into()),
        });
        let (choices, rows) = matching(&options, "astra");
        assert_eq!(
            rows,
            [
                Row::Heading("OpenAI".into()),
                Row::Choice(0),
                Row::Heading("OpenCode Go".into()),
                Row::Choice(1),
            ],
        );
        assert_eq!(
            choices,
            [
                MatchedChoice { option: 0, row: 1 },
                MatchedChoice { option: 3, row: 3 },
            ],
        );
        let (choices, rows) = matching(&options, "opencode flash");
        assert_eq!(choices, [MatchedChoice { option: 2, row: 1 }]);
        assert_eq!(rows, [Row::Heading("OpenCode Go".into()), Row::Choice(0)]);
    }
}
