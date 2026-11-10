//! One open session as a self-contained view: the thread, pending requests,
//! notices and the composer. Interactive chat tabs stream their session's
//! events; saved child documents reuse the renderer without a writer/pump.
//! Each [`Chat`] tells the workspace what matters
//! outside through [`ChatEvent`].

mod actions;
mod activity_dock;
mod attachment_io;
mod attachments;
mod codex;
mod composer;
mod context_requests;
pub(super) mod context_usage;
mod draft;
mod feedback;
mod history;
mod images;
mod notice;
mod pump;
mod question_card;
pub(in crate::ui) mod readonly;
mod replies;
mod requests;
mod row_layout;
mod scroller;
mod session;
#[cfg(test)]
mod snapshot_tests;
mod stamp;
mod stream_batch;
pub(in crate::ui) use stream_batch::yield_after_batch;
mod thread;
mod tool_chain;
mod tool_hint;
pub(super) mod tool_row;
mod turn_copy;
mod turn_footer;
mod user_input;
mod view;

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::{
    Theme,
    input::{InputEvent, TextareaState},
};
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
    SessionStateChanged(u64),
    SessionEntriesChanged(crate::transcript::Change, u64),
    /// A run of this session finished.
    RunSettled,
    /// Background read projections changed the saved session list.
    CatalogUpdated,
    /// Open this exact saved subagent conversation beside its parent.
    OpenSubagent {
        child_thread_id: String,
        session: proteus_contracts::app_protocol::AppSubagentSession,
    },
    /// May arrive after the parent run ended and its progress row was folded
    /// into canonical history. Saved child documents still need the final tail.
    SubagentLifecycle {
        child_thread_id: String,
        status: String,
    },
    /// Activity of some session; every session stream carries all of them.
    Activity {
        session_dir: PathBuf,
        activity: AppSessionActivity,
    },
    /// The user sent a message, so the chat is worth keeping open.
    Sent,
}

/// All resolved font metrics used by prose, native code and tool controls.
/// Theme color changes alone do not discard measurements.
#[derive(Clone, PartialEq)]
struct RowFonts {
    chat: SharedString,
    interface: SharedString,
    mono: SharedString,
    interface_size: Pixels,
    mono_size: Pixels,
}

impl RowFonts {
    fn current(cx: &App) -> Self {
        let theme = Theme::global(cx);
        Self {
            chat: prefs::chat_font(cx),
            interface: theme.font_family.clone(),
            mono: theme.mono_font_family.clone(),
            interface_size: theme.font_size,
            mono_size: theme.mono_font_size,
        }
    }
}

fn row_sizes_changed(before: ViewPreferences, after: ViewPreferences) -> bool {
    before.scale != after.scale
        || before.chat_text != after.chat_text
        || before.chat_width != after.chat_width
        || before.compact_tools != after.compact_tools
}

impl ChatEvent {
    // Context::emit queues effects FIFO. Keep the same originating event ID
    // when several Workspaces project this shared Chat into an extension.
    fn projection_id() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    fn state_changed() -> Self {
        Self::SessionStateChanged(Self::projection_id())
    }
    fn entries_changed(change: crate::transcript::Change) -> Self {
        Self::SessionEntriesChanged(change, Self::projection_id())
    }
}

pub(super) struct Chat {
    api: Option<Api>,
    codex: Option<codex::Binding>,
    readonly: Option<readonly::Binding>,
    workspace: WeakEntity<crate::ui::workspace::Workspace>,
    pub session: Session,
    /// Invalidates a delayed catalog when another config snapshot was applied.
    config_generation: u64,
    context_requests: context_requests::ContextRequests,
    history: history::History,
    scroller: Entity<ChatScroller>,
    composer: Entity<TextareaState>,
    /// A separate view owns animation ticks so they never repaint the thread.
    thinking_caption: Entity<super::motion::Shimmer>,
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
    /// Last represented row and settlement state whose timestamp was measured.
    stamp_tail: Option<(u64, bool)>,
    turn_copy: turn_footer::CopyState,
    /// Approval and question cards that just arrived, for enter motion.
    fresh_requests: Recent,
    user_input: UserInputDrafts,
    /// Retained selection of the common strip above the input.
    activity_dock: activity_dock::DockState,
    /// Chat-bound fixed contributions; inactive tabs release their processes.
    extension_activities: HashMap<&'static str, crate::ui::extensions::ActivityContribution>,
    notice: Notice,
    extras: ComposerExtras,
    /// Current owner; asynchronous replies follow a transferred tab.
    window_handle: AnyWindowHandle,
    _composer_events: Subscription,
    /// Sizes the row heights were measured with.
    view: ViewPreferences,
    row_fonts: RowFonts,
    enter_sends: bool,
    _pump: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ChatEvent> for Chat {}

pub(in crate::ui) fn invalidate_height_cache() {
    scroller::invalidate_height_cache();
}

/// The chat owns renderer/font policy; its scroller only adds physical width.
fn height_layout_fingerprint(cx: &App) -> [u8; 32] {
    let fonts = RowFonts::current(cx);
    let view = prefs::current(cx).view;
    // Source stamps were collected by extension discovery already.
    scroller::height_hash(format!(
        "scalar-v1:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{:?}",
        env!("PROTEUS_BUILD_COMMIT"),
        fonts.chat,
        fonts.interface,
        fonts.mono,
        f32::from(fonts.interface_size).to_bits(),
        f32::from(fonts.mono_size).to_bits(),
        view.scale,
        view.chat_text,
        view.chat_width,
        view.compact_tools,
        crate::version::LABEL,
        crate::ui::extensions::scalar_layout_fingerprint(cx)
    ))
}

impl Chat {
    pub fn new(
        api: Api,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        session_dir: PathBuf,
        prepared_stream: Option<async_channel::Receiver<crate::connection::PumpItem>>,
        extras: ComposerExtras,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let events = prepared_stream
            .unwrap_or_else(|| crate::connection::spawn_pump(api.clone(), session_dir.clone()));
        let initial = crate::connection::take_initial_events(&events);
        let mut chat = Self::base(
            Some(api),
            workspace,
            Session::new(session_dir),
            extras,
            window,
            cx,
        );
        chat._pump = Some(pump::start(events, cx));
        chat.apply_batch(initial, cx);
        chat.load_config(cx);
        chat
    }

    fn base(
        api: Option<Api>,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        session: Session,
        extras: ComposerExtras,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let autoscroll = prefs::current(cx).view.autoscroll;
        let scroller = cx.new(|_| ChatScroller::new(autoscroll));
        let enter_sends = prefs::current(cx).send == SendMode::Enter;
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 12)
                .placeholder("Сообщение для Proteus")
                .submit_on_enter(enter_sends)
        });
        let subscriptions = vec![
            cx.subscribe(&scroller, |this, _, event: &scroller::HistoryEdge, cx| {
                this.load_history(event.0, cx)
            }),
            cx.subscribe(&scroller, |this, _, _: &scroller::LatestRequested, cx| {
                this.show_latest(cx)
            }),
            cx.subscribe(&scroller, |this, _, event: &scroller::HistorySeek, cx| {
                this.seek_history(event.0, cx)
            }),
            cx.subscribe(
                &scroller,
                |this, _, _: &scroller::HistorySeekCanceled, cx| this.cancel_history_seek(cx),
            ),
            cx.observe_global::<ClientPreferences>(|this, cx| {
                let before = this.extension_activities.len();
                this.extension_activities.retain(|id, _| {
                    crate::ui::extensions::is_enabled(id, cx)
                        && crate::ui::extensions::find(id).is_some_and(|manifest| manifest.activity)
                });
                if before != this.extension_activities.len() {
                    cx.notify();
                }
                let enter_sends = prefs::current(cx).send == SendMode::Enter;
                if this.enter_sends != enter_sends {
                    this.enter_sends = enter_sends;
                    this.composer
                        .update(cx, |state, cx| state.set_submit_on_enter(enter_sends, cx));
                }
                this.refresh_row_metrics(cx);
            }),
            // System appearance and resolved theme fonts can change without
            // a ClientPreferences write (including a different mono family).
            cx.observe_global::<Theme>(|this, cx| this.refresh_row_metrics(cx)),
        ];
        let composer_events = Self::subscribe_composer(&composer, window, cx);
        Self {
            api,
            workspace,
            codex: None,
            readonly: None,
            session,
            config_generation: 0,
            context_requests: context_requests::ContextRequests::default(),
            history: history::History::default(),
            scroller,
            composer,
            thinking_caption: cx.new(|_| super::motion::Shimmer::new("Думаю")),
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
            stamp_tail: None,
            turn_copy: Default::default(),
            fresh_requests: Recent::default(),
            user_input: UserInputDrafts::default(),
            activity_dock: activity_dock::DockState::default(),
            extension_activities: HashMap::new(),
            notice: Notice::default(),
            extras,
            window_handle: window.window_handle(),
            _composer_events: composer_events,
            view: prefs::current(cx).view,
            row_fonts: RowFonts::current(cx),
            enter_sends,
            _pump: None,
            _subscriptions: subscriptions,
        }
    }

    fn refresh_row_metrics(&mut self, cx: &mut Context<Self>) {
        let view = prefs::current(cx).view;
        let fonts = RowFonts::current(cx);
        let remeasure = row_sizes_changed(self.view, view) || self.row_fonts != fonts;
        let autoscroll_changed = self.view.autoscroll != view.autoscroll;
        let appearance_changed = self.view.surface_radius != view.surface_radius;
        self.view = view;
        self.row_fonts = fonts;
        if remeasure || autoscroll_changed {
            self.scroller.update(cx, |scroller, cx| {
                if autoscroll_changed {
                    scroller.set_autoscroll(view.autoscroll, cx);
                }
                if remeasure {
                    scroller.remeasure(cx);
                }
            });
        }
        if appearance_changed {
            cx.notify();
        }
    }

    /// Retain the session stream, transcript, input and native content while
    /// moving their window ownership. No second chat or backend is created.
    pub(in crate::ui) fn rehost(
        &mut self,
        workspace: WeakEntity<crate::ui::workspace::Workspace>,
        extras: ComposerExtras,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window_handle = window.window_handle();
        self.extension_activities.clear();
        self.workspace = workspace.clone();
        self.extras = extras;
        self.composer
            .update(cx, |composer, cx| composer.rehost(window, cx));
        self._composer_events = Self::subscribe_composer(&self.composer, window, cx);
        self.replies.rehost(workspace, window, cx);
        if let Some(binding) = &mut self.codex {
            for input in binding.answers.values() {
                input.update(cx, |state, cx| state.rehost(window, cx));
            }
        }
        self.scroller
            .update(cx, |scroller, cx| scroller.remeasure(cx));
        cx.notify();
    }

    fn subscribe_composer(
        composer: &Entity<TextareaState>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            &composer,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if this.readonly.is_some() {
                    return;
                }
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
                let empty =
                    this.composer.read(cx).value().trim().is_empty() && this.attachments.is_empty();
                if !(empty && this.confirm_pending(window, cx)) {
                    this.send(window, cx);
                }
            },
        )
    }

    pub(in crate::ui) fn supports_core(&self) -> bool {
        self.api.is_some() && self.readonly.is_none()
    }

    fn archived(&self) -> bool {
        self.codex.as_ref().map_or_else(
            || self.history.archived(),
            |binding| binding.archive.is_some(),
        )
    }

    /// Nothing was said or typed here yet, so the chat can go unnoticed.
    pub fn is_blank(&self, cx: &App) -> bool {
        self.session.transcript.entries().is_empty()
            && self.session.feedback.visible_sends().next().is_none()
            && !self.session.running()
            && self.composer.read(cx).value().trim().is_empty()
            && self.attachments.is_empty()
            && !self.attachments_loading
            && self.saved_drafts.is_empty()
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        if let Some(binding) = &self.readonly {
            binding.focus.focus(window, cx);
            return;
        }
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
        // Read-only documents deliberately cannot run session commands. Their
        // history/images use explicit read APIs outside this command helper.
        if self.readonly.is_some() {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
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
        self.readonly.as_ref().map_or_else(
            || self.composer.focus_handle(cx),
            |binding| binding.focus.clone(),
        )
    }
}

#[cfg(test)]
mod metric_tests {
    use super::{ViewPreferences, row_sizes_changed};

    #[test]
    fn scrolling_policy_and_corner_style_preserve_visited_row_sizes() {
        let before = ViewPreferences::default();
        let mut after = before;
        after.autoscroll = !before.autoscroll;
        after.surface_radius = 0;
        assert!(!row_sizes_changed(before, after));

        // Independent geometry changes must still invalidate sizes even
        // when the non-geometric settings changed in the same update.
        after.chat_text += 1;
        assert!(row_sizes_changed(before, after));
        after.chat_text = before.chat_text;
        after.compact_tools = !before.compact_tools;
        assert!(row_sizes_changed(before, after));
    }
}
