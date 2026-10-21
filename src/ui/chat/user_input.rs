use std::collections::HashMap;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use proteus_contracts::app_protocol::AppPendingRequests;
use proteus_contracts::contracts::{UserInputAnswer, UserInputRequest, UserInputResponse};

use super::Chat;

/// Answers being composed for pending `request_user_input` calls, one
/// question of a request on screen at a time.
#[derive(Default)]
pub(super) struct UserInputDrafts {
    selected: HashMap<String, HashMap<String, Vec<String>>>,
    texts: HashMap<(String, String), (Entity<InputState>, Subscription)>,
    pages: HashMap<String, usize>,
}

impl UserInputDrafts {
    /// Drops drafts of requests the server no longer waits for.
    pub fn retain(&mut self, pending: &AppPendingRequests) {
        let live = |id: &str| {
            pending
                .user_inputs
                .iter()
                .any(|request| request.request_id == id)
        };
        self.selected.retain(|request, _| live(request));
        self.texts.retain(|(request, _), _| live(request));
        self.pages.retain(|request, _| live(request));
    }

    /// The question of `request` on screen.
    pub fn page(&self, request: &UserInputRequest) -> usize {
        let last = request.questions.len().saturating_sub(1);
        self.pages
            .get(&request.request_id)
            .copied()
            .unwrap_or(0)
            .min(last)
    }

    pub fn set_page(&mut self, request: &str, page: usize) {
        self.pages.insert(request.to_owned(), page);
    }

    pub fn is_selected(&self, request: &str, question: &str, label: &str) -> bool {
        self.selected
            .get(request)
            .and_then(|answers| answers.get(question))
            .is_some_and(|labels| labels.iter().any(|selected| selected == label))
    }

    pub fn toggle(&mut self, request: &str, question: &str, label: &str, multi_select: bool) {
        let labels = self
            .selected
            .entry(request.to_owned())
            .or_default()
            .entry(question.to_owned())
            .or_default();
        match labels.iter().position(|selected| selected == label) {
            Some(index) => {
                labels.remove(index);
            }
            None if multi_select => labels.push(label.to_owned()),
            None => *labels = vec![label.to_owned()],
        }
    }

    /// Forgets the answer to one question, e.g. when it is skipped.
    pub fn clear(&mut self, request: &str, question: &str, window: &mut Window, cx: &mut App) {
        if let Some(answers) = self.selected.get_mut(request) {
            answers.remove(question);
        }
        if let Some((input, _)) = self.texts.get(&(request.to_owned(), question.to_owned())) {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// The free-text answer field; Enter in it moves on like "Далее".
    pub fn text_input(
        &mut self,
        request: &str,
        question: &str,
        secret: bool,
        window: &mut Window,
        cx: &mut Context<Chat>,
    ) -> Entity<InputState> {
        self.texts
            .entry((request.to_owned(), question.to_owned()))
            .or_insert_with(|| {
                let input = cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder("Или напишите свой ответ")
                        .masked(secret)
                });
                let request = request.to_owned();
                let subscription = cx.subscribe_in(
                    &input,
                    window,
                    move |chat, _, event: &InputEvent, window, cx| {
                        if let InputEvent::PressEnter { .. } = event {
                            chat.next_question(&request, window, cx);
                        }
                    },
                );
                (input, subscription)
            })
            .0
            .clone()
    }

    fn values(&self, request: &str, question: &str, cx: &App) -> Vec<String> {
        let mut values = self
            .selected
            .get(request)
            .and_then(|answers| answers.get(question))
            .cloned()
            .unwrap_or_default();
        if let Some((input, _)) = self.texts.get(&(request.to_owned(), question.to_owned())) {
            let text = input.read(cx).value().trim().to_owned();
            if !text.is_empty() {
                values.push(text);
            }
        }
        values
    }

    pub fn is_answered(&self, request: &str, question: &str, cx: &App) -> bool {
        !self.values(request, question, cx).is_empty()
    }

    /// The answered questions; skipped ones are left out.
    pub fn response(&self, request: &UserInputRequest, cx: &App) -> UserInputResponse {
        let answers = request
            .questions
            .iter()
            .filter_map(|question| {
                let values = self.values(&request.request_id, &question.id, cx);
                (!values.is_empty()).then(|| (question.id.clone(), UserInputAnswer::new(values)))
            })
            .collect();
        UserInputResponse::new(answers)
    }
}
