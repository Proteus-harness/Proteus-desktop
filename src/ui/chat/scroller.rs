//! The chat's virtual list with its own tail following.
//!
//! gpui's `FollowMode::Tail` snaps to the end on every layout. Here the list
//! stays in normal mode and the scroller glides toward the end itself, so a
//! streaming answer scrolls smoothly and following can be switched off.
//!
//! Following stops when the user moves up (the offset drops while content
//! remains below) and resumes once the view reaches the end again.
//!
//! While a disclosure changes height, an empty tail keeps the list from
//! clamping its scroll position to the smaller content. As in Settings, the
//! freed room stays blank below the content until scrolling up consumes it.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::scroll::{ScrollableElement as _, ScrollableMask};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ui::floor::TailFloor;
use crate::ui::kit;
use crate::ui::kit::Hint as _;
use crate::ui::motion;

mod scrollbar;
use scrollbar::HistoryScrollbar;
pub(super) use scrollbar::{HistoryRange, ScrollTarget};

const OVERDRAW: Pixels = px(400.);
/// Time constant of the glide: the remaining distance shrinks by ~63% per τ.
const GLIDE_SECONDS: f32 = 0.08;
/// Distances longer than this many viewports jump instead of gliding.
const JUMP_VIEWPORTS: f32 = 2.;
const SETTLED: Pixels = px(0.5);
const EDGE: Pixels = px(8.);
/// Frames past a disclosure's motion during which the list still holds.
const HOLD_TAIL: Duration = Duration::from_millis(50);

pub(super) struct ChatScroller {
    list: ListState,
    scrollbar: HistoryScrollbar,
    following: bool,
    autoscroll: bool,
    /// Offset seen on the previous frame, to tell the user's scrolling apart.
    last_offset: Option<Pixels>,
    last_frame: Option<Instant>,
    /// Content changed since the last frame; the new size is known only after
    /// the next layout.
    dirty: bool,
    /// Size of the viewport on the last layout.
    size: Option<gpui_kit::Size<Pixels>>,
    hold: Option<Hold>,
    tail: TailFloor,
    history_edges: (bool, bool),
    archived: bool,
}

pub(super) struct HistoryEdge(pub super::history::Direction);
impl EventEmitter<HistoryEdge> for ChatScroller {}
pub(super) struct LatestRequested;
impl EventEmitter<LatestRequested> for ChatScroller {}
pub(super) struct HistorySeek(pub ScrollTarget);
impl EventEmitter<HistorySeek> for ChatScroller {}
pub(super) struct HistorySeekCanceled;
impl EventEmitter<HistorySeekCanceled> for ChatScroller {}

/// Rows whose height animates after the user toggled them.
struct Hold {
    rows: Range<usize>,
    until: Instant,
}

impl ChatScroller {
    pub(super) fn new(autoscroll: bool, cx: &mut Context<Self>) -> Self {
        // A permanent empty final row owns retained room. Large bottom
        // padding cannot do this: gpui skips row prepaint if the padding
        // itself fills the viewport.
        let list = ListState::new(1, ListAlignment::Top, OVERDRAW);
        let scrollbar = HistoryScrollbar::new(list.clone());
        let wheel = scrollbar.clone();
        let weak = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            let canceled = wheel.cancel_target();
            let weak = weak.clone();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |_, cx| {
                    if canceled {
                        cx.emit(HistorySeekCanceled);
                    }
                    cx.notify();
                });
            });
        });
        Self {
            list,
            scrollbar,
            following: true,
            autoscroll,
            last_offset: None,
            last_frame: None,
            dirty: false,
            size: None,
            hold: None,
            tail: TailFloor::default(),
            history_edges: (false, false),
            archived: false,
        }
    }

    pub(super) fn set_autoscroll(&mut self, autoscroll: bool, cx: &mut Context<Self>) {
        if self.autoscroll == autoscroll {
            return;
        }
        self.autoscroll = autoscroll;
        self.following = autoscroll && self.tail.height() <= SETTLED && self.gap() <= SETTLED;
        cx.notify();
    }

    pub(super) fn splice(&mut self, range: Range<usize>, count: usize, cx: &mut Context<Self>) {
        let total = self.row_count();
        if range.start > range.end || range.end > total {
            return;
        }
        // A loaded or reloaded thread opens at its end: the list would show
        // the top, and with rows above still unmeasured the glide would crawl
        // through the whole history.
        let whole = range.start == 0 && range.end == total;
        if whole {
            self.hold = None;
            self.clear_tail();
        }
        self.list.splice(range, count);
        self.history_edges = (false, false);
        self.last_offset = None;
        if whole && self.following {
            self.list.scroll_to_end();
            self.last_offset = None;
        }
        self.changed(cx);
    }

    pub(super) fn set_archived(&mut self, archived: bool, cx: &mut Context<Self>) {
        if archived != self.archived {
            self.archived = archived;
            if archived {
                self.following = false;
            } else {
                self.list.remeasure();
            }
            self.last_offset = None;
            cx.notify();
        }
    }

    pub(super) fn visible_rows(&self) -> Range<usize> {
        let start = self.list.logical_scroll_top().item_ix.min(self.row_count());
        let bottom = self.list.viewport_bounds().bottom();
        let mut end = start;
        while end < self.row_count() {
            let Some(bounds) = self.list.bounds_for_item(end) else {
                break;
            };
            if bounds.top() >= bottom {
                break;
            }
            end += 1;
        }
        start..end.max(start.saturating_add(1)).min(self.row_count())
    }

    pub(super) fn set_history_range(
        &mut self,
        range: Option<HistoryRange>,
        seeking: bool,
        cx: &mut Context<Self>,
    ) {
        if self.scrollbar.set_range(range, seeking) {
            cx.notify();
        }
    }

    pub(super) fn scroll_to_history(&mut self, target: ScrollTarget, cx: &mut Context<Self>) {
        self.following = false;
        self.hold = None;
        self.clear_tail();
        self.last_offset = None;
        self.scrollbar.scroll_to(target);
        // Resident positions also supersede any older page request.
        cx.emit(HistorySeek(target));
        self.changed(cx);
    }

    pub(super) fn history_ready(&mut self, cx: &mut Context<Self>) {
        self.history_edges = (false, false);
        cx.notify();
    }

    pub(super) fn append(&mut self, count: usize, cx: &mut Context<Self>) {
        let end = self.row_count();
        self.splice(end..end, count, cx);
    }

    pub(super) fn remeasure(&mut self, cx: &mut Context<Self>) {
        self.list.remeasure();
        self.changed(cx);
    }

    pub(super) fn remeasure_items(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if range.start > range.end || range.end > self.row_count() {
            return;
        }
        self.list.remeasure_items(range);
        self.changed(cx);
    }

    /// Rows the user just opened or closed: they are measured on every frame
    /// anyway while visible, and once more after their motion, so rows that
    /// left the viewport mid-way do not keep a height from the middle. The
    /// list stops following meanwhile: the row stays under the pointer and
    /// grows downwards instead of being pulled up after the end.
    pub(super) fn toggled(&mut self, rows: Range<usize>, cx: &mut Context<Self>) {
        self.following = false;
        if self.tail.hold(self.list.viewport_bounds().size.height) {
            self.remeasure_tail();
        }
        self.hold = Some(Hold {
            rows: rows.clone(),
            until: Instant::now() + motion::SLOW + HOLD_TAIL,
        });
        self.remeasure_items(rows, cx);
    }

    /// Bring the latest row into view; follows again when autoscroll is on.
    pub(super) fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.hold = None;
        self.clear_tail();
        self.following = true;
        self.dirty = true;
        cx.notify();
    }

    /// A new width re-measures every row and keeps the top one in place, so
    /// the end would drop out of view; a new height (the composer growing,
    /// a request card) keeps the top too and cuts the last lines until the
    /// glide brings them back. A following list stays at its end. Called
    /// before the list lays out at `size`.
    fn keep_end(&mut self, size: gpui_kit::Size<Pixels>) {
        let resized = self.size.replace(size).is_some_and(|last| last != size);
        if resized && self.following {
            self.list.scroll_to_end();
            // Rows above are unmeasured now, so the offset reads lower
            // without the user scrolling.
            self.last_offset = None;
        }
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.dirty = true;
        cx.notify();
    }

    /// Pixels to the scrollable end, including retained blank room.
    fn gap(&self) -> Pixels {
        let max = self.list.max_offset_for_scrollbar().y;
        let offset = -self.list.scroll_px_offset_for_scrollbar().y;
        (max - offset).max(px(0.))
    }

    fn scrolled_up(&self) -> bool {
        // Collapsing a row retains blank room to keep the viewport steady.
        // Only actual content below the viewport warrants a jump button.
        self.archived
            || !self.following
                && self.row_count().checked_sub(1).is_some_and(|last| {
                    self.list.bounds_for_item(last).map_or_else(
                        || self.gap() > self.tail.height() + SETTLED,
                        |bounds| {
                            // Item bounds omit the list's top padding. Prefer
                            // actual row geometry to the frozen scrollbar range.
                            bounds.bottom() + EDGE > self.list.viewport_bounds().bottom() + SETTLED
                        },
                    )
                })
    }

    /// Advance following by one frame; returns whether another frame is needed.
    fn step(&mut self, now: Instant) -> bool {
        let positioning = self.scrollbar.settle();
        let offset = -self.list.scroll_px_offset_for_scrollbar().y;
        let gap = self.gap();
        let dt = self
            .last_frame
            .map_or(1. / 60., |at| now.duration_since(at).as_secs_f32().min(0.1));
        self.last_frame = Some(now);

        // Content growth leaves the offset alone and an item collapsing above
        // lowers both; only the user moves up while room stays below.
        if let Some(last) = self.last_offset
            && offset < last - px(1.)
            && gap > SETTLED
        {
            self.following = false;
        }
        let dirty = std::mem::take(&mut self.dirty);
        if self.hold(now) {
            return true;
        }
        let tail_changed = self.settle_tail();
        if !self.following
            && gap <= SETTLED
            && self.tail.height() <= SETTLED
            && self.autoscroll
            && !self.archived
            && !self.list.is_scrollbar_dragging()
        {
            self.following = true;
        }

        let mut running = dirty || tail_changed || positioning;
        if self.following && gap > SETTLED {
            let viewport = self.list.viewport_bounds().size.height;
            if !motion::plays() || gap > viewport * JUMP_VIEWPORTS {
                self.list.scroll_to_end();
                // The end anchor reads past the content until laid out.
                self.last_frame = Some(now);
                self.last_offset = None;
                return true;
            } else {
                let share = 1. - (-dt / GLIDE_SECONDS).exp();
                self.list.scroll_by((gap * share).max(px(1.)).min(gap));
            }
            running = true;
        }
        if self.following && !self.autoscroll && !dirty && gap <= SETTLED {
            // A jump without autoscroll ends once the end is reached.
            self.following = false;
        }
        self.last_offset = Some(-self.list.scroll_px_offset_for_scrollbar().y);
        if !running {
            self.last_frame = None;
        }
        running
    }
}

impl ChatScroller {
    fn row_count(&self) -> usize {
        self.list.item_count().saturating_sub(1)
    }

    fn remeasure_tail(&self) {
        let tail = self.row_count();
        self.list.remeasure_items(tail..tail + 1);
    }

    fn clear_tail(&mut self) {
        if self.tail.clear() {
            self.remeasure_tail();
        }
    }

    fn settle_tail(&mut self) -> bool {
        if self.tail.height() <= px(0.) {
            return false;
        }
        let Some(tail) = self.list.bounds_for_item(self.row_count()) else {
            // The list stopped measuring before reaching its last row:
            // natural content still fills the viewport without this tail.
            self.clear_tail();
            return true;
        };
        // bounds_for_item excludes list padding. Both edges count toward
        // filling its viewport, just as the page padding counts in Floor.
        let visible = self.list.viewport_bounds().bottom() - tail.top() - EDGE * 2.;
        let changed = self.tail.settle(visible);
        if changed {
            self.remeasure_tail();
        }
        changed
    }

    /// One frame of a held list; false once nothing is held. The list does
    /// not scroll while held.
    fn hold(&mut self, now: Instant) -> bool {
        let Some(hold) = &self.hold else {
            return false;
        };
        if now >= hold.until {
            let rows = hold.rows.clone();
            self.hold = None;
            if rows.end <= self.row_count() {
                self.list.remeasure_items(rows);
            }
            self.dirty = true;
            return false;
        }
        self.last_offset = Some(-self.list.scroll_px_offset_for_scrollbar().y);
        self.last_frame = Some(now);
        true
    }
}

type RenderRow = Box<dyn FnMut(usize, &mut Window, &mut App) -> AnyElement>;

/// The list element; rows come from the caller's renderer.
#[derive(IntoElement)]
pub(super) struct ChatList {
    id: ElementId,
    state: Entity<ChatScroller>,
    render_row: RenderRow,
}

impl ChatList {
    pub(super) fn new(
        id: impl Into<ElementId>,
        state: Entity<ChatScroller>,
        render_row: impl FnMut(usize, &mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            state,
            render_row: Box::new(render_row),
        }
    }
}

impl RenderOnce for ChatList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if self.state.update(cx, |state, cx| {
            if let Some(target) = state.scrollbar.take_target() {
                state.scroll_to_history(target, cx);
            }
            let running = state.step(Instant::now());
            let visible = state.visible_rows();
            let edges = (
                !state.following && visible.start < 8,
                state.archived && visible.end.saturating_add(8) >= state.row_count(),
            );
            if edges.0 && !state.history_edges.0 {
                cx.emit(HistoryEdge(super::history::Direction::Older));
            } else if edges.1 && !state.history_edges.1 {
                cx.emit(HistoryEdge(super::history::Direction::Newer));
            }
            state.history_edges = edges;
            running
        }) {
            window.request_animation_frame();
        }
        let (list_state, scrollbar, scrolled_up, tail_row, tail_height) = {
            let state = self.state.read(cx);
            (
                state.list.clone(),
                state.scrollbar.clone(),
                state.scrolled_up(),
                state.row_count(),
                state.tail.height(),
            )
        };
        let mut render_row = self.render_row;
        let rows = list(list_state.clone(), move |ix, window, cx| {
            if ix == tail_row {
                return div().w_full().h(tail_height).into_any_element();
            }
            div()
                .w_full()
                .min_w_0()
                .child(render_row(ix, window, cx))
                .into_any_element()
        })
        .size_full()
        .min_h_0()
        .py(EDGE);
        let state = self.state.clone();
        let resized = self.state.clone();
        div()
            .id(self.id.clone())
            .relative()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .id((self.id.clone(), "viewport"))
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    // Ahead of the rows, so it runs before the list lays out.
                    .on_prepaint(move |bounds, _, cx| {
                        resized.update(cx, |state, _| state.keep_end(bounds.size))
                    })
                    .child(rows)
                    .vertical_scrollbar(&scrollbar),
            )
            .child(ScrollableMask::new(Axis::Vertical, &list_state).id(self.id.clone()))
            .when(scrolled_up, |this| {
                this.child(
                    h_flex()
                        .absolute()
                        .left_0()
                        .right_0()
                        .bottom_3()
                        .justify_center()
                        .child(motion::rise_in(
                            kit::button((self.id.clone(), "jump"))
                                .secondary()
                                .icon(IconName::ArrowDown)
                                .hint("К последним сообщениям", cx)
                                .rounded(cx.theme().radius_full())
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .on_click(move |_, _, cx| {
                                    state.update(cx, |_, cx| cx.emit(LatestRequested));
                                }),
                            (self.id, "jump-enter"),
                        )),
                )
            })
    }
}
