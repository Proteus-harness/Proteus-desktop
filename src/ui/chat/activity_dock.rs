// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (render_activity_bar, plan summary and pending permission controls).
//! One retained strip attached to the top of the composer. Public extension
//! places, context, pending requests and the real plan use one compact surface.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::requests::{approval_icon, request_fresh};
use super::tool_row::plan_list;
use super::{Chat, ComposerSlot};
use crate::ui::kit::{self, ButtonLabel as _};
use crate::ui::{fade, motion, prefs};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ActivityId {
    Approval(String),
    Question(String),
    Plan,
}

impl ActivityId {
    fn key(&self, chat: EntityId) -> SharedString {
        let suffix = match self {
            Self::Approval(id) => format!("approval-{id}"),
            Self::Question(id) => format!("question-{id}"),
            Self::Plan => "plan".to_owned(),
        };
        format!("composer-activity-{chat}-{suffix}").into()
    }

    fn pending(&self) -> bool {
        !matches!(self, Self::Plan)
    }
}

#[derive(Default)]
pub(super) struct DockState {
    live: Vec<ActivityId>,
    selected: Option<ActivityId>,
}

impl DockState {
    /// An arriving request opens without replacing the answer being edited.
    /// Explicitly collapsing an existing request remains retained.
    fn sync(&mut self, live: Vec<ActivityId>) {
        let fresh = live
            .iter()
            .find(|id| id.pending() && !self.live.contains(id));
        let selected_pending = self
            .selected
            .as_ref()
            .is_some_and(|id| id.pending() && live.contains(id));
        let vanished = self.selected.as_ref().is_some_and(|id| !live.contains(id));
        if !selected_pending && fresh.is_some() {
            self.selected = fresh.cloned();
        } else if vanished {
            self.selected = live.iter().find(|id| id.pending()).cloned();
        }
        self.live = live;
    }

    pub fn selected(&self) -> Option<&ActivityId> {
        self.selected.as_ref()
    }

    pub fn pending_target(&self) -> Option<ActivityId> {
        self.selected
            .as_ref()
            .filter(|id| id.pending())
            .or_else(|| self.live.iter().find(|id| id.pending()))
            .cloned()
    }

    fn toggle(&mut self, id: ActivityId) {
        self.selected = (self.selected.as_ref() != Some(&id)).then_some(id);
    }
}

struct Activity {
    id: ActivityId,
    label: String,
    icon: Lucide,
}

impl Chat {
    pub(super) fn render_activity_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let max_details = prefs::scaled(360.)
            .to_pixels(window.rem_size())
            .min(window.viewport_size().height * 0.4);
        let plan = self.active_plan();
        let mut activities = Vec::new();
        if let Some(pending) = &self.session.pending {
            activities.extend(pending.approvals.iter().map(|request| Activity {
                id: ActivityId::Approval(request.approval_id.clone()),
                label: format!("Approval · {}", request.call.name),
                icon: approval_icon(request),
            }));
            activities.extend(pending.user_inputs.iter().map(|request| {
                Activity {
                    id: ActivityId::Question(request.request_id.clone()),
                    label: request
                        .title
                        .clone()
                        .filter(|title| !title.trim().is_empty())
                        .or_else(|| {
                            request
                                .questions
                                .first()
                                .map(|question| question.header.clone())
                                .filter(|title| !title.trim().is_empty())
                        })
                        .unwrap_or_else(|| "Вопрос".to_owned()),
                    icon: Lucide::MessageCircleQuestionMark,
                }
            }));
        }
        if let Some(plan) = &plan {
            activities.push(Activity {
                id: ActivityId::Plan,
                label: format!("План {}/{}", plan.done, plan.steps.len()),
                icon: Lucide::ListTodo,
            });
        }
        let old = self.activity_dock.selected.clone();
        self.activity_dock
            .sync(activities.iter().map(|item| item.id.clone()).collect());
        self.animate_dock_change(old, cx);
        // Auto-open is decided after Chat's normal animation driver.
        motion::drive(window, cx);
        let selected = self.activity_dock.selected.clone();
        let mut triggers = Vec::new();
        let mut details = Vec::new();
        for activity in activities {
            let key = activity.id.key(cx.entity_id());
            let open = selected.as_ref() == Some(&activity.id);
            let id = activity.id.clone();
            let trigger = kit::button(key.clone())
                .icon(activity.icon)
                .faded_label(activity.label)
                .quiet(cx)
                .xsmall()
                .selected(open)
                .toggled(open)
                .max_w(prefs::scaled(240.))
                .when(open, |button| {
                    button
                        .bg(theme.sidebar_accent)
                        .text_color(theme.sidebar_accent_foreground)
                })
                .child(motion::chevron(
                    SharedString::from(format!("{key}-chevron")),
                    open,
                    theme.muted_foreground,
                ))
                .on_click(cx.listener(move |this, _, window, cx| {
                    let old = this.activity_dock.selected.clone();
                    this.activity_dock.toggle(id.clone());
                    this.animate_dock_change(old, cx);
                    this.focus(window, cx);
                    cx.notify();
                }));
            triggers.push(
                motion::rise_in_if(
                    div().child(trigger),
                    SharedString::from(format!("{key}-arrive")),
                    request_fresh(&activity.id, self),
                )
                .into_any_element(),
            );
            if !open && !motion::moving(&key, open, cx) {
                continue;
            }
            let content = match activity.id {
                ActivityId::Plan => plan.as_ref().map(|plan| {
                    div()
                        .p_3()
                        .child(plan_list(&plan.steps, cx))
                        .into_any_element()
                }),
                ref id => self.render_request(id, window, cx),
            };
            if let Some(content) = content {
                details.extend(motion::reveal(key.clone(), open, cx, || {
                    div()
                        .id(SharedString::from(format!("{key}-details")))
                        .w_full()
                        .max_h(max_details)
                        .overflow_y_scroll()
                        .border_b_1()
                        .border_color(theme.border)
                        .when(!open, |body| {
                            body.capture_any_mouse_down(|_, _, cx| cx.stop_propagation())
                                .capture_any_mouse_up(|_, _, cx| cx.stop_propagation())
                        })
                        .child(content)
                        .into_any_element()
                }));
            }
        }
        let start = (self.extras)(cx.entity_id(), ComposerSlot::Start, cx);
        let end = (self.extras)(cx.entity_id(), ComposerSlot::End, cx);
        // Keep the strip's own geometry independent of selection. The
        // composer is bottom-anchored: details grow above this stable row.
        let summary = plan
            .as_ref()
            .and_then(|plan| plan.current())
            .map(str::to_owned);
        v_flex()
            .w_full()
            .children(details)
            .child(
                h_flex()
                    .id("composer-activity-strip")
                    .w_full()
                    .min_w_0()
                    .flex_wrap()
                    .max_h(prefs::scaled(96.))
                    .overflow_y_scroll()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .children(start)
                    .children(triggers)
                    .children(summary.map(|summary| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_w(prefs::scaled(320.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(fade::faded(summary))
                    }))
                    .child(h_flex().ml_auto().gap_1().children(end)),
            )
            .into_any_element()
    }

    fn animate_dock_change(&self, previous: Option<ActivityId>, cx: &mut Context<Self>) {
        if previous == self.activity_dock.selected {
            return;
        }
        if let Some(previous) = previous {
            motion::toggled(previous.key(cx.entity_id()), false, cx);
        }
        if let Some(selected) = &self.activity_dock.selected {
            motion::toggled(selected.key(cx.entity_id()), true, cx);
        }
    }

    /// Empty-composer Enter first reveals a folded request. Approval is
    /// dispatched only after its details have actually been visible.
    pub(super) fn open_pending_activity(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(ActivityId, bool)> {
        let id = self.activity_dock.pending_target()?;
        let open = self.activity_dock.selected.as_ref() == Some(&id);
        if !open {
            let old = self.activity_dock.selected.replace(id.clone());
            self.animate_dock_change(old, cx);
            cx.notify();
        }
        Some((id, open))
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivityId, DockState};
    fn approval(id: &str) -> ActivityId {
        ActivityId::Approval(id.into())
    }
    fn question(id: &str) -> ActivityId {
        ActivityId::Question(id.into())
    }

    #[::core::prelude::v1::test]
    fn pending_arrival_selection_and_completion_preserve_the_answer_being_edited() {
        let mut state = DockState::default();
        state.sync(vec![ActivityId::Plan]);
        assert_eq!(state.selected(), None);
        state.toggle(ActivityId::Plan);
        state.sync(vec![approval("a"), question("q"), ActivityId::Plan]);
        assert_eq!(state.selected(), Some(&approval("a")));
        state.toggle(question("q"));
        state.sync(vec![
            approval("a"),
            approval("b"),
            question("q"),
            ActivityId::Plan,
        ]);
        assert_eq!(state.selected(), Some(&question("q")));
        state.sync(vec![approval("b"), ActivityId::Plan]);
        assert_eq!(state.selected(), Some(&approval("b")));
        state.sync(vec![ActivityId::Plan]);
        assert_eq!(state.selected(), None);
    }

    #[::core::prelude::v1::test]
    fn folded_requests_stay_folded_but_keyboard_target_keeps_stable_identity() {
        let mut state = DockState::default();
        state.sync(vec![approval("a"), approval("b")]);
        state.toggle(approval("b"));
        assert_eq!(state.pending_target(), Some(approval("b")));
        state.toggle(approval("b"));
        state.sync(vec![approval("a"), approval("b")]);
        assert_eq!(state.selected(), None);
        assert_eq!(state.pending_target(), Some(approval("a")));
        state.sync(vec![approval("b"), approval("c")]);
        assert_eq!(state.selected(), Some(&approval("c")));
    }
}
