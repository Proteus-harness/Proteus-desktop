//! Local status is real list content, while disclosure retention is only a
//! minimum height. Neither changes persisted row indexes or history counts.

use gpui_kit::*;

use super::{ChatScroller, SETTLED};
use crate::ui::prefs;

#[derive(Clone, Default, PartialEq, Eq)]
pub(in crate::ui::chat) struct FooterState {
    pub thinking: bool,
    /// Receipt text and images are immutable for an id. Only the short
    /// identity/status/count are needed to invalidate its measured layout.
    pub receipt: Option<(String, &'static str, usize)>,
}

impl FooterState {
    pub(super) fn visible(&self) -> bool {
        self.thinking || self.receipt.is_some()
    }
}

impl ChatScroller {
    pub(super) fn set_reading_gap(&mut self, gap: Pixels) -> bool {
        if self.reading_gap == gap {
            return false;
        }
        self.reading_gap = gap;
        self.wheel.invalidate(&self.list);
        self.remeasure_tail();
        self.last_offset = None;
        self.dirty = true;
        true
    }

    /// Blank room shares the tail's minimum with real status content. It
    /// never warrants a jump button by itself, even before exact row bounds
    /// are available during a history seek or the first layout.
    pub(super) fn trailing_blank(&self) -> Pixels {
        let status = if self.footer.visible() {
            self.footer_height
        } else {
            px(0.)
        };
        self.tail.height().max(status + self.reading_gap) - status
    }

    pub(super) fn update_footer(&mut self, footer: FooterState) -> bool {
        if self.footer == footer {
            return false;
        }
        self.footer = footer;
        self.wheel.invalidate(&self.list);
        self.remeasure_tail();
        self.dirty = true;
        true
    }

    pub(super) fn measure_footer(&mut self, height: Pixels) -> bool {
        // The retained tail is a minimum on the same row, not empty room
        // added after the footer. A freshly measured footer can consume that
        // entire minimum even when its own height has not changed. Clearing
        // it changes no row geometry, so no list mutation is needed during
        // prepaint; it only releases the guard against resuming following.
        let consumed_floor = height + self.reading_gap >= self.tail.height() && self.tail.clear();
        let resized = (self.footer_height - height).abs() > SETTLED;
        if !resized && !consumed_floor {
            return false;
        }
        self.footer_height = height;
        // A receipt being replaced by its canonical row can shorten the
        // footer. Its layout adjustment is not an upward wheel gesture.
        if resized {
            self.last_offset = None;
        }
        self.dirty = true;
        true
    }
}

/// Only drawn when content continues below the viewport. At the natural end
/// there is no mask over the final line, timestamp or thinking caption.
/// The scroll viewport reaches the input. Reading space lives in its final
/// row, so the full gradient remains at the input edge when browsing history.
pub(super) fn edge_fade(background: Hsla) -> Div {
    div()
        .absolute()
        .bottom_0()
        .left_0()
        .right_0()
        .h(prefs::scaled(32.))
        .bg(linear_gradient(
            180.,
            linear_color_stop(background.opacity(0.), 0.),
            linear_color_stop(background, 1.),
        ))
}
