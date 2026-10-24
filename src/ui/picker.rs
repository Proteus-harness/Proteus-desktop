// Адаптировано из Zed: crates/picker/src/picker.rs (query, selection and reveal-selected scrolling)
//! A searchable, virtualized single-choice control. Callers supply labels and
//! values; keyboard navigation, filtering, focus and selection belong here.

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

use super::{fade, prefs, selection};

const ROW: f32 = 28.;
const VISIBLE: usize = 8;

#[derive(Clone, PartialEq)]
pub(super) struct Choice<T> {
    pub label: SharedString,
    pub value: T,
    pub selected: bool,
}

type Pick<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

pub(super) struct Picker<T: Clone + PartialEq + 'static> {
    search: Entity<InputState>,
    searchable: bool,
    list_focus: FocusHandle,
    options: Vec<Choice<T>>,
    matches: Vec<usize>,
    cursor: usize,
    empty: SharedString,
    placeholder: SharedString,
    scroll: UniformListScrollHandle,
    pick: Pick<T>,
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
            list_focus: cx.focus_handle(),
            options,
            matches: Vec::new(),
            cursor: 0,
            empty,
            placeholder,
            scroll: UniformListScrollHandle::new(),
            pick: Rc::new(pick),
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

    pub fn focus(&self, cx: &App) -> FocusHandle {
        if self.searchable {
            self.search.read(cx).focus_handle(cx)
        } else {
            self.list_focus.clone()
        }
    }

    fn current(&self) -> Option<&Choice<T>> {
        self.matches
            .get(self.cursor)
            .and_then(|i| self.options.get(*i))
    }

    fn filter(&mut self, keep: Option<T>, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.matches = matching(&self.options, &query);
        self.cursor = keep
            .as_ref()
            .and_then(|value| {
                self.matches
                    .iter()
                    .position(|i| &self.options[*i].value == value)
            })
            .or_else(|| {
                query
                    .is_empty()
                    .then(|| self.matches.iter().position(|i| self.options[*i].selected))
                    .flatten()
            })
            .unwrap_or(0);
        self.scroll
            .scroll_to_item(self.cursor, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn keys(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        match event.keystroke.key.as_str() {
            "up" | "down" | "pageup" | "pagedown" if !self.matches.is_empty() => {
                let count = self.matches.len();
                self.cursor = match event.keystroke.key.as_str() {
                    "up" => (self.cursor + count - 1) % count,
                    "down" => (self.cursor + 1) % count,
                    "pageup" => self.cursor.saturating_sub(VISIBLE),
                    _ => (self.cursor + VISIBLE).min(count - 1),
                };
                self.scroll
                    .scroll_to_item(self.cursor, ScrollStrategy::Nearest);
                cx.notify();
            }
            "enter" => {
                if let Some(choice) = self.current() {
                    (self.pick)(choice.value.clone(), window, cx);
                }
            }
            // Esc first clears the local filter. The next Esc can dismiss
            // the containing popup, without selecting or changing anything.
            "escape" if !self.search.read(cx).value().is_empty() => {
                self.search
                    .update(cx, |search, cx| search.set_value("", window, cx));
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

fn matching<T>(options: &[Choice<T>], query: &str) -> Vec<usize> {
    let terms: Vec<_> = query.split_whitespace().collect();
    options
        .iter()
        .enumerate()
        .filter_map(|(i, option)| {
            let label = option.label.to_lowercase();
            terms.iter().all(|term| label.contains(term)).then_some(i)
        })
        .collect()
}

impl<T: Clone + PartialEq + 'static> Render for Picker<T> {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.matches.len();
        let focused = self.focus(cx).is_focused(window);
        let rows = uniform_list(
            "picker-rows",
            count,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|row| {
                        let option = &this.options[this.matches[row]];
                        let cursor = row == this.cursor;
                        let value = option.value.clone();
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
                            .when(option.selected, |row| {
                                row.text_color(cx.theme().sidebar_accent_foreground)
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(fade::caption("caption", option.label.clone())),
                            )
                            .when(cursor && focused && super::kit::keyboard(), |row| {
                                row.child(selection::focus(cx))
                            })
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.cursor = row;
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
            .tab_stop(!self.searchable)
            .w_full()
            .min_w_0()
            .gap_1()
            .capture_key_down(cx.listener(Self::keys))
            .when(self.searchable, |body| {
                body.child(super::search::field(&self.search).small())
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
            })
            .collect();
        assert_eq!(matching(&options, "openai обыч"), vec![0]);
        assert_eq!(matching(&options, "обычная"), vec![0, 1]);
        assert!(matching(&options, "none").is_empty());
        assert_eq!(matching(&options, ""), vec![0, 1, 2]);
    }
}
