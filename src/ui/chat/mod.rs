//! One open session as a self-contained view: the thread, pending requests,
//! notices and the composer. Every chat tab owns a [`Chat`]; it streams its
//! session's events while it lives and tells the workspace what matters
//! outside through [`ChatEvent`].

mod actions;
mod activity_dock;
mod attachment_io;
mod attachments;
mod composer;
mod context_requests;
pub(super) mod context_usage;
mod draft;
mod fences;
mod images;
mod notice;
mod plan_panel;
mod pump;
mod question_card;
mod replies;
mod requests;
mod scroller;
mod session;
mod stamp;
mod thread;
mod tool_chain;
pub(super) mod tool_row;
mod user_input;
mod view;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, TextareaState};
use gpui_kit::*;
use proteus_contracts::app_protocol::AppSessionActivity;

pub(super) use session::{ChatState, Session};

use super::motion::Recent;
use super::prefs::{self, ClientPreferences};
use crate::api::Api;
use crate::preferences::{SendMode, ViewPreferences};
use attachments::Attachments;
use draft::SavedDraft;
use images::SentImages;
use notice::Notice;
use replies::Replies;
use scroller::ChatScroller;
use user_input::UserInputDrafts;

/// Content the host adds around the composer, e.g. extension icons. It gets
/// the chat's entity id, so the host can show it in one chat only.
pub(super) type ComposerExtras = Rc<dyn Fn(EntityId, ComposerSlot, &App) -> Option<AnyElement>>;

/// Where host content goes around the composer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ComposerSlot {
    /// The left extension section of the activity strip above the input.
    Start,
    /// The right extension section of the activity strip above the input.
    End,
}

pub(super) enum ChatEvent {
    SessionStateChanged,
    SessionEntriesChanged(crate::transcript::Change),
    /// A run of this session finished.
    RunSettled,
    /// Background read projections changed the saved session list.
    CatalogUpdated,
    /// Activity of some session; every session stream carries all of them.
    Activity {
        session_dir: PathBuf,
        activity: AppSessionActivity,
    },
    /// The user sent a message, so the chat is worth keeping open.
    Sent,
    /// The app server shut down.
    ServerStopped,
}

pub(super) struct Chat {
    api: Api,
    workspace: WeakEntity<crate::ui::workspace::Workspace>,
    pub session: Session,
    /// Invalidates a delayed catalog when another config snapshot was applied.
    config_generation: u64,
    context_requests: context_requests::ContextRequests,
    scroller: Entity<ChatScroller>,
    composer: Entity<TextareaState>,
    composer_strip_bounds: Rc<Cell<Bounds<Pixels>>>,
    attachments: Attachments,
    attachments_loading: bool,
    /// A delayed picker/read must not land in a newer composer draft.
    attachment_generation: u64,
    /// Failed sends stay available even if a newer draft occupies the input.
    saved_drafts: Vec<SavedDraft>,
    sent_images: SentImages,
    replies: Replies,
    expanded: HashSet<u64>,
    /// Individual result/argument blocks the user asked to see in full.
    full_blocks: HashSet<(u64, usize)>,
    /// Shared diff state survives folding and virtualized chat rows.
    tool_diffs: HashMap<u64, Entity<crate::ui::diff::DiffView>>,
    /// Folded tool runs the user opened, by the key of their first row.
    open_chains: HashSet<u64>,
    /// Rows pushed by live events, for enter motion.
    fresh_rows: Recent,
    /// Approval and question cards that just arrived, for enter motion.
    fresh_requests: Recent,
    user_input: UserInputDrafts,
    /// Retained selection of the common strip above the input.
    activity_dock: activity_dock::DockState,
    notice: Notice,
    extras: ComposerExtras,
    /// Sizes the row heights were measured with.
    view: ViewPreferences,
    _pump: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChatEvent> for Chat {}

impl Chat {
    pub fn new(
        api: Api,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        session_dir: PathBuf,
        extras: ComposerExtras,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let autoscroll = prefs::current(cx).view.autoscroll;
        let scroller = cx.new(|cx| ChatScroller::new(autoscroll, cx));
        let enter_sends = prefs::current(cx).send == SendMode::Enter;
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 12)
                .placeholder("Сообщение для Proteus")
                .submit_on_enter(enter_sends)
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                        return;
                    }
                    let InputEvent::PressEnter { secondary, shift } = *event else {
                        return;
                    };
                    let sends = match prefs::current(cx).send {
                        SendMode::Enter => !shift,
                        SendMode::CtrlEnter => secondary,
                    };
                    if !sends {
                        return;
                    }
                    let empty = this.composer.read(cx).value().trim().is_empty()
                        && this.attachments.is_empty();
                    if !(empty && this.confirm_pending(window, cx)) {
                        this.send(window, cx);
                    }
                },
            ),
            cx.observe_global::<ClientPreferences>(|this, cx| {
                let enter_sends = prefs::current(cx).send == SendMode::Enter;
                this.composer
                    .update(cx, |state, cx| state.set_submit_on_enter(enter_sends, cx));
                // Row heights are cached; sizes change with them.
                let view = prefs::current(cx).view;
                if this.view != view {
                    this.view = view;
                    this.scroller.update(cx, |scroller, cx| {
                        scroller.set_autoscroll(view.autoscroll, cx);
                        scroller.remeasure(cx);
                    });
                }
            }),
        ];
        let pump = pump::start(api.clone(), session_dir.clone(), cx);
        let mut chat = Self {
            api,
            workspace,
            session: Session::new(session_dir),
            config_generation: 0,
            context_requests: context_requests::ContextRequests::default(),
            scroller,
            composer,
            composer_strip_bounds: Rc::default(),
            attachments: Attachments::default(),
            attachments_loading: false,
            attachment_generation: 0,
            saved_drafts: Vec::new(),
            sent_images: SentImages::default(),
            replies: Replies::default(),
            expanded: HashSet::new(),
            full_blocks: HashSet::new(),
            tool_diffs: HashMap::new(),
            open_chains: HashSet::new(),
            fresh_rows: Recent::default(),
            fresh_requests: Recent::default(),
            user_input: UserInputDrafts::default(),
            activity_dock: activity_dock::DockState::default(),
            notice: Notice::default(),
            extras,
            view: prefs::current(cx).view,
            _pump: pump,
            _subscriptions: subscriptions,
        };
        chat.load_config(cx);
        chat
    }

    /// Nothing was said or typed here yet, so the chat can go unnoticed.
    pub fn is_blank(&self, cx: &App) -> bool {
        self.session.transcript.entries().is_empty()
            && !self.session.running()
            && self.composer.read(cx).value().trim().is_empty()
            && self.attachments.is_empty()
            && !self.attachments_loading
            && self.saved_drafts.is_empty()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }

    /// Runs a blocking server call off the UI thread; failures become the
    /// chat's notice.
    fn call<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        job: impl FnOnce(&Api) -> anyhow::Result<R> + Send + 'static,
        done: impl FnOnce(&mut Self, R, &mut Context<Self>) + 'static,
    ) {
        let api = self.api.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { job(&api) })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(value) => done(this, value, cx),
                    Err(error) => this.show_notice(format!("{error:#}"), cx),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Focusable for Chat {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.composer.focus_handle(cx)
    }
}
