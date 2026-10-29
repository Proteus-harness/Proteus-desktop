//! Adjacent assistant deltas retain their original envelopes and ordered effects.
use proteus_contracts::app_protocol::AppServerEvent;
use proteus_contracts::domain::{Event, EventEnvelope, MessageId};
use proteus_contracts::model_standard::MessagePhase;

use super::{Change, Transcript};

/// Only compatible text events can enter a run. Every other event is a barrier.
#[cfg_attr(test, derive(Clone))]
pub(crate) struct TextRun {
    events: Vec<Box<AppServerEvent>>,
}

impl TextRun {
    pub fn new(event: Box<AppServerEvent>) -> Result<Self, Box<AppServerEvent>> {
        if delta(&event).is_none() {
            return Err(event);
        }
        Ok(Self {
            events: vec![event],
        })
    }

    pub fn accepts(&self, event: &AppServerEvent) -> bool {
        let Some(first) = self.events.first().and_then(|event| delta(event)) else {
            return false;
        };
        let Some(next) = delta(event) else {
            return false;
        };
        first.envelope.schema_version == next.envelope.schema_version
            && first.envelope.session_id == next.envelope.session_id
            && first.envelope.thread_id == next.envelope.thread_id
            && first.envelope.turn_id == next.envelope.turn_id
            && first.message_id == next.message_id
            && first.phase == next.phase
    }

    pub fn push(&mut self, event: Box<AppServerEvent>) {
        assert!(self.accepts(&event), "incompatible text run event");
        self.events.push(event);
    }

    #[cfg(test)]
    pub(crate) fn into_events(self) -> Vec<Box<AppServerEvent>> {
        self.events
    }

    /// Keep the real cursor gate and its source order, including rejected events.
    pub fn retain(&mut self, mut accept: impl FnMut(&AppServerEvent) -> bool) {
        self.events.retain(|event| accept(event));
    }
}

/// Original per-event effects remain observable even when layout work is reduced.
pub(crate) struct TextRunOutcome {
    pub(crate) feedback_changed: bool,
    changes: Vec<Change>,
}

impl TextRunOutcome {
    pub fn event_changes(&self) -> impl Iterator<Item = Change> + '_ {
        self.changes
            .iter()
            .copied()
            .filter(|change| *change != Change::None)
    }

    pub fn layout_changes(&self) -> impl Iterator<Item = Change> + '_ {
        // Creation must survive even if a later empty delta truncates the row.
        let change = if self.changes.contains(&Change::Pushed) {
            Some(Change::Pushed)
        } else {
            self.event_changes().last()
        };
        change.into_iter()
    }

    pub fn shown(&self) -> bool {
        self.feedback_changed || self.event_changes().next().is_some()
    }
}

struct Delta<'a> {
    envelope: &'a EventEnvelope,
    message_id: MessageId,
    phase: Option<MessagePhase>,
    offset: usize,
    text: &'a str,
}

fn delta(event: &AppServerEvent) -> Option<Delta<'_>> {
    let AppServerEvent::Runtime { envelope } = event else {
        return None;
    };
    let Event::AssistantTextDelta {
        offset,
        message_id,
        phase,
        text,
    } = &envelope.event
    else {
        return None;
    };
    Some(Delta {
        envelope,
        message_id: *message_id,
        phase: *phase,
        offset: *offset,
        text,
    })
}

impl Transcript {
    pub(crate) fn apply_text_run(&mut self, run: &TextRun) -> TextRunOutcome {
        let Some(first) = run.events.first().and_then(|event| delta(event)) else {
            return TextRunOutcome {
                changes: Vec::new(),
                feedback_changed: false,
            };
        };
        // Do not add a tail buffer to the common singleton arrival path.
        if run.events.len() == 1 {
            let change = self.apply_runtime(first.envelope.thread_id, &first.envelope.event);
            self.stamp(change, first.envelope.timestamp_ms);
            return TextRunOutcome {
                changes: vec![change],
                feedback_changed: false,
            };
        }
        if !self.is_turn_thread(first.envelope.thread_id) {
            return TextRunOutcome {
                changes: vec![Change::None; run.events.len()],
                feedback_changed: false,
            };
        }
        #[cfg(test)]
        {
            self.text_work.delta_lookups += 1;
        }
        let index = self
            .entries
            .iter()
            .rposition(|entry| entry.message_id == Some(first.message_id));
        let base = index.map(|index| self.entries[index].text.clone());
        let mut edit = DeltaEdit::new(base.as_deref().unwrap_or(""));
        let row = index.unwrap_or(self.entries.len());
        let mut exists = index.is_some();
        let mut created_at = None;
        let mut changes = Vec::with_capacity(run.events.len());
        for event in &run.events {
            let item = delta(event).expect("validated text event");
            let change = if !exists {
                if item.text.is_empty() {
                    Change::None
                } else {
                    // A first nonempty delta creates a row regardless of offset.
                    edit.apply(0, item.text);
                    exists = true;
                    created_at = Some(item.envelope.timestamp_ms);
                    Change::Pushed
                }
            } else {
                edit.apply(item.offset, item.text);
                Change::Updated(row)
            };
            changes.push(change);
        }
        if !exists {
            return TextRunOutcome {
                changes,
                feedback_changed: false,
            };
        }
        #[cfg(test)]
        {
            self.text_work.prior_row_prefix_bytes += edit.keep;
            self.text_work.delta_row_replacements += 1;
        }
        let text = edit.finish();
        if let Some(index) = index {
            let entry = &mut self.entries[index];
            entry.text = text;
            entry.streaming = true;
        } else {
            self.stop_streaming();
            let key = self.allocate_key();
            self.entries.push(super::Entry {
                key,
                message_id: Some(first.message_id),
                role: super::Role::Assistant,
                text,
                images: Vec::new(),
                tool: None,
                subagent: None,
                streaming: true,
                time: created_at,
            });
        }
        TextRunOutcome {
            changes,
            feedback_changed: false,
        }
    }
}

/// A composition of the existing UTF-8 byte-offset splice rule, without
/// copying the unchanged base until the run is materialized.
struct DeltaEdit<'a> {
    base: &'a str,
    keep: usize,
    tail: String,
}

impl<'a> DeltaEdit<'a> {
    fn new(base: &'a str) -> Self {
        Self {
            base,
            keep: base.len(),
            tail: String::new(),
        }
    }

    fn apply(&mut self, offset: usize, text: &str) {
        let length = self.keep.checked_add(self.tail.len()).expect("text length");
        let valid = if offset <= self.keep {
            self.base.is_char_boundary(offset)
        } else {
            offset <= length && self.tail.is_char_boundary(offset - self.keep)
        };
        let keep = if valid { offset } else { length };
        if keep <= self.keep {
            self.keep = keep;
            self.tail.clear();
        } else {
            self.tail.truncate(keep - self.keep);
        }
        self.tail.push_str(text);
    }

    fn finish(self) -> std::sync::Arc<str> {
        let length = self.keep.checked_add(self.tail.len()).expect("text length");
        let mut text = String::with_capacity(length);
        text.push_str(&self.base[..self.keep]);
        text.push_str(&self.tail);
        text.into()
    }
}

/// Deterministic logical work, not wall time or allocator/RSS measurements.
#[cfg(test)]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TextWork {
    pub delta_lookups: usize,
    /// Prior row-prefix bytes copied; excludes tail buffers and Arc conversion.
    pub prior_row_prefix_bytes: usize,
    /// Final logical delta row-text replacements, not allocation counts.
    pub delta_row_replacements: usize,
}

#[cfg(test)]
impl Transcript {
    pub(crate) fn take_text_work(&mut self) -> TextWork {
        std::mem::take(&mut self.text_work)
    }
}

#[cfg(test)]
mod tests;
