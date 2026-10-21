use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::input::Input;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::contracts::{UserInputQuestion, UserInputRequest};

use super::Chat;
use super::requests::panel_card;
use crate::ui::kit;
use crate::ui::kit::Hint as _;
use crate::ui::kit::Pressable as _;

impl Chat {
    /// A `request_user_input` call, one question at a time: options with
    /// their numbers, a free answer, skipping and paging.
    pub(super) fn render_user_input(
        &mut self,
        request: &UserInputRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let rid = request.request_id.clone();
        let count = request.questions.len();
        let page = self.user_input.page(request);
        let Some(question) = request.questions.get(page) else {
            return div().into_any_element();
        };
        let last = page + 1 == count;
        let answered = self.user_input.is_answered(&rid, &question.id, cx);
        let input = self
            .user_input
            .text_input(&rid, &question.id, question.is_secret, window, cx);
        let title = request
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .or_else(|| (!question.header.trim().is_empty()).then(|| question.header.clone()))
            .unwrap_or_else(|| "Вопрос".to_owned());

        let pager = (count > 1).then(|| {
            let (back, next) = (rid.clone(), rid.clone());
            h_flex()
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    kit::button(SharedString::from(format!("question-back-{rid}")))
                        .icon(IconName::ChevronLeft)
                        .ghost()
                        .xsmall()
                        .disabled(page == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.user_input.set_page(&back, page.saturating_sub(1));
                            cx.notify();
                        })),
                )
                .child(format!("{} из {count}", page + 1))
                .child(
                    kit::button(SharedString::from(format!("question-next-{rid}")))
                        .icon(IconName::ChevronRight)
                        .ghost()
                        .xsmall()
                        .disabled(last)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.user_input.set_page(&next, page + 1);
                            cx.notify();
                        })),
                )
        });
        let close_id = rid.clone();
        let header = h_flex()
            .gap_2()
            .text_sm()
            .child(
                Icon::new(Lucide::MessageCircleQuestionMark)
                    .small()
                    .text_color(theme.muted_foreground),
            )
            .child(div().flex_1().min_w_0().font_medium().child(title))
            .children(pager)
            .child(
                kit::button(SharedString::from(format!("question-close-{rid}")))
                    .icon(IconName::Close)
                    .hint("Закрыть без ответа", cx)
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.submit_user_input(&close_id, true, cx)
                    })),
            );

        let options = question
            .options
            .iter()
            .enumerate()
            .map(|(ix, option)| self.render_option(&rid, question, ix, option, cx))
            .collect::<Vec<_>>();

        let (skip, go) = (rid.clone(), rid.clone());
        let footer = h_flex()
            .gap_2()
            .child(
                Icon::new(Lucide::Pencil)
                    .xsmall()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&input).appearance(false).small()),
            )
            .child(
                kit::button(SharedString::from(format!("question-skip-{rid}")))
                    .label("Пропустить")
                    .outline()
                    .small()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.skip_question(&skip, window, cx)
                    })),
            )
            .child(
                kit::button(SharedString::from(format!("question-go-{rid}")))
                    .label(if last {
                        "Отправить"
                    } else {
                        "Далее"
                    })
                    .primary()
                    .small()
                    .disabled(!answered)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.next_question(&go, window, cx)),
                    ),
            );

        panel_card(cx)
            .child(header)
            .child(div().text_sm().child(question.question.clone()))
            .when(!options.is_empty(), |card| {
                card.child(v_flex().gap_0p5().children(options))
            })
            .child(footer)
            .into_any_element()
    }

    fn render_option(
        &self,
        rid: &str,
        question: &UserInputQuestion,
        ix: usize,
        option: &proteus_contracts::contracts::UserInputQuestionOption,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selected = self
            .user_input
            .is_selected(rid, &question.id, &option.label);
        let (request, qid, label) = (rid.to_owned(), question.id.clone(), option.label.clone());
        let multi = question.multi_select;
        let badge = div()
            .flex_none()
            .size_5()
            .flex()
            .items_center()
            .justify_center()
            .rounded(theme.radius)
            .border_1()
            .text_xs()
            .map(|badge| {
                if selected {
                    badge
                        .border_color(theme.primary)
                        .bg(theme.primary)
                        .text_color(theme.primary_foreground)
                } else {
                    badge
                        .border_color(theme.border)
                        .text_color(theme.muted_foreground)
                }
            })
            .child((ix + 1).to_string());
        h_flex()
            .id(SharedString::from(format!(
                "answer-{rid}-{}-{ix}",
                question.id
            )))
            .gap_3()
            .px_2()
            .py_1p5()
            .rounded(theme.radius)
            .cursor_pointer()
            .pressable(cx)
            .when(selected, |row| row.bg(theme.accent))
            .hover(|row| row.bg(theme.list_hover))
            .child(badge)
            .child(
                v_flex()
                    .min_w_0()
                    .child(div().text_sm().child(option.label.clone()))
                    .children((!option.description.trim().is_empty()).then(|| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(option.description.clone())
                    })),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.user_input.toggle(&request, &qid, &label, multi);
                cx.notify();
            }))
            .into_any_element()
    }

    /// "Далее": the next question, or sending after the last one. An
    /// unanswered question stays.
    pub(super) fn next_question(&mut self, rid: &str, _: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.user_input_request(rid) else {
            return;
        };
        let page = self.user_input.page(request);
        let Some(question) = request.questions.get(page) else {
            return;
        };
        if !self.user_input.is_answered(rid, &question.id, cx) {
            return;
        }
        self.advance(rid, page, request.questions.len(), cx);
    }

    /// A digit typed into the empty composer picks that option of the open
    /// question, as its number badge shows.
    pub(super) fn pick_option(
        &mut self,
        digit: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let composer = self.composer.read(cx);
        if !composer.value().is_empty() || !composer.focus_handle(cx).is_focused(window) {
            return false;
        }
        let Some(pending) = self.session.pending.as_ref() else {
            return false;
        };
        let Some(request) = pending
            .user_inputs
            .first()
            .filter(|_| pending.approvals.is_empty())
        else {
            return false;
        };
        let Some(question) = request.questions.get(self.user_input.page(request)) else {
            return false;
        };
        let Some(option) = digit.checked_sub(1).and_then(|ix| question.options.get(ix)) else {
            return false;
        };
        let (rid, qid, label) = (
            request.request_id.clone(),
            question.id.clone(),
            option.label.clone(),
        );
        let multi = question.multi_select;
        self.user_input.toggle(&rid, &qid, &label, multi);
        cx.notify();
        true
    }

    /// "Пропустить": the question goes unanswered.
    fn skip_question(&mut self, rid: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(request) = self.user_input_request(rid) else {
            return;
        };
        let page = self.user_input.page(request);
        let count = request.questions.len();
        let Some(question) = request.questions.get(page) else {
            return;
        };
        let qid = question.id.clone();
        self.user_input.clear(rid, &qid, window, cx);
        self.advance(rid, page, count, cx);
    }

    fn advance(&mut self, rid: &str, page: usize, count: usize, cx: &mut Context<Self>) {
        if page + 1 < count {
            self.user_input.set_page(rid, page + 1);
            cx.notify();
        } else {
            self.submit_user_input(rid, false, cx);
        }
    }
}
