//! Client-side transcript: the server snapshot plus live runtime events folded
//! the same way the server folds turn progress, so that the next snapshot
//! replaces it without visible jumps.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use proteus_contracts::app_protocol::{
    AppTranscriptMessage, AppTranscriptSubagent, AppTranscriptTool,
};
use proteus_contracts::domain::{Event, ImageRef, MessageId, ThreadId, ToolResult};
use serde_json::Value;

const MAX_SUBAGENT_TOOLS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    User,
    Assistant,
    Reasoning,
    System,
}

#[derive(Clone, Debug)]
pub struct Entry<Tool = Arc<AppTranscriptTool>, Child = Arc<AppTranscriptSubagent>> {
    /// Stable client identity, reused across snapshots for the same message.
    pub key: u64,
    pub message_id: Option<MessageId>,
    pub role: Role,
    pub text: Arc<str>,
    pub images: Vec<ImageRef>,
    /// Display windows share the frozen payload until a live event changes it.
    pub tool: Option<Tool>,
    pub subagent: Option<Child>,
    pub streaming: bool,
    /// Creation time in Unix milliseconds. Canonical history timestamps take
    /// precedence over provisional live/local event times.
    pub time: Option<i64>,
}

/// What a fold step changed, for list virtualization.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    None,
    Spliced {
        start: usize,
        removed: usize,
        inserted: usize,
    },
    Pushed,
    Updated(usize),
}

#[derive(Default, Clone)]
pub struct Transcript {
    entries: Vec<Entry>,
    external_ids: HashMap<(String, String), u64>,
    turn_thread: Option<ThreadId>,
    /// A known snapshot root remains authoritative over child TurnStarted events.
    snapshot_root_known: bool,
    next_key: Arc<AtomicU64>,
    #[cfg(test)]
    text_work: text_run::TextWork,
}

impl Transcript {
    /// A snapshot replaces routing too: its TurnStarted need not be replayed.
    pub fn set_root_thread(&mut self, root: Option<ThreadId>) {
        self.turn_thread = root;
        self.snapshot_root_known = root.is_some();
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Independent rows in windows of this transcript share one key space.
    pub(crate) fn empty_window(&self) -> Self {
        Self {
            next_key: Arc::clone(&self.next_key),
            ..Self::default()
        }
    }

    /// Replace the transcript with a server snapshot. Rows that did not change
    /// keep their key and text allocation, so the change is reported as a
    /// splice from the first differing row instead of a full reset.
    pub fn reset(&mut self, messages: Vec<AppTranscriptMessage>) -> Change {
        let old = std::mem::take(&mut self.entries);
        // The index borrows old row IDs. Consume each winner exactly once,
        // including when the incoming snapshot contains duplicate identities.
        let previous: HashMap<_, _> = old
            .iter()
            .filter_map(|entry| Some((identity_of(entry)?, Cell::new(Some(entry)))))
            .collect();
        let mut entries = Vec::with_capacity(messages.len());
        let mut local_rows = old.as_slice();
        for (index, message) in messages.into_iter().enumerate() {
            let role = role_of(&message.role);
            let reused = match identity(
                message.message_id,
                role,
                message.tool.as_ref(),
                message.subagent.as_ref(),
            ) {
                Some(id) => previous.get(&id).and_then(Cell::take).map(|before| {
                    // Compare with the owned wire text before allocating another Arc.
                    let shared =
                        (before.text.as_ref() == message.text).then(|| before.text.clone());
                    (before.key, shared, before.time)
                }),
                // Rows without identity are matched by position and content.
                None => old
                    .get(index)
                    .filter(|before| {
                        identity_of(before).is_none()
                            && before.role == role
                            && before.text.as_ref() == message.text
                    })
                    .map(|before| (before.key, Some(before.text.clone()), before.time)),
            };
            let entry = if let Some((key, shared, time)) = reused {
                let mut entry = self.entry_from_with_text(message, role, shared);
                entry.key = key;
                entry.time = entry.time.or(time);
                entry
            } else {
                let mut entry = self.entry_from_with_text(message, role, None);
                entry.time = entry.time.or_else(|| local_time_of(&mut local_rows, &entry));
                entry
            };
            entries.push(entry);
        }
        let start = old
            .iter()
            .zip(&entries)
            .position(|(before, after)| !same_row(before, after))
            .unwrap_or(old.len().min(entries.len()));
        let change = Change::Spliced {
            start,
            removed: old.len() - start,
            inserted: entries.len() - start,
        };
        self.entries = entries;
        match change {
            Change::Spliced {
                removed: 0,
                inserted: 0,
                ..
            } => Change::None,
            change => change,
        }
    }

    pub fn prepend(&mut self, messages: Vec<AppTranscriptMessage>) -> Change {
        let inserted = messages.len();
        if inserted == 0 {
            return Change::None;
        }
        let next_key = &self.next_key;
        let incoming = messages.into_iter().map(|message| {
            let role = role_of(&message.role);
            let key = next_key.fetch_add(1, Ordering::Relaxed) + 1;
            Entry::from_wire(key, message, role, None)
        });
        // Eviction leaves reusable storage in a full paging window. Only
        // reuse an exact fit; a much larger former window must still be freed.
        if self.entries.capacity() - self.entries.len() == inserted {
            drop(self.entries.splice(..0, incoming));
        } else {
            let mut prefix: Vec<_> = incoming.collect();
            prefix.append(&mut self.entries);
            self.entries = prefix;
        }
        Change::Spliced {
            start: 0,
            removed: 0,
            inserted,
        }
    }

    /// Insert a page, sharing identities with the live tail where they overlap.
    pub fn append_page(&mut self, messages: Vec<AppTranscriptMessage>, known: &Self) -> Change {
        let start = self.entries.len();
        // As in a forward scan, the first live row wins for a duplicate identity.
        let live: HashMap<_, _> = known
            .entries
            .iter()
            .rev()
            .filter_map(|entry| Some((identity_of(entry)?, entry)))
            .collect();
        for message in messages {
            let role = role_of(&message.role);
            let old = identity(
                message.message_id,
                role,
                message.tool.as_ref(),
                message.subagent.as_ref(),
            )
            .and_then(|id| live.get(&id).copied());
            let shared = old
                .filter(|old| old.text.as_ref() == message.text)
                .map(|old| Arc::clone(&old.text));
            let mut entry = self.entry_from_with_text(message, role, shared);
            if let Some(old) = old {
                entry.key = old.key;
                entry.time = entry.time.or(old.time);
            }
            self.entries.push(entry);
        }
        Change::Spliced {
            start,
            removed: 0,
            inserted: self.entries.len() - start,
        }
    }

    pub fn remove(&mut self, range: std::ops::Range<usize>) -> Change {
        let change = Change::Spliced {
            start: range.start,
            removed: range.len(),
            inserted: 0,
        };
        self.entries.drain(range);
        change
    }

    pub fn append_known(&mut self, entries: &[Entry]) -> Change {
        let start = self.entries.len();
        self.entries.extend_from_slice(entries);
        Change::Spliced {
            start,
            removed: 0,
            inserted: entries.len(),
        }
    }

    /// Complete a display window using the same state as a clone of the
    /// live transcript, but never clone live rows already planned for eviction.
    pub(crate) fn complete_history_window(&mut self, live: &Self, keep: usize) {
        debug_assert!(Arc::ptr_eq(&self.next_key, &live.next_key));
        self.entries.extend_from_slice(&live.entries[..keep]);
        self.external_ids.clone_from(&live.external_ids);
        self.turn_thread = live.turn_thread;
        self.snapshot_root_known = live.snapshot_root_known;
        #[cfg(test)]
        self.text_work.clone_from(&live.text_work);
    }

    pub fn push_user(
        &mut self,
        message_id: Option<MessageId>,
        text: &str,
        images: Vec<ImageRef>,
    ) -> Change {
        self.stop_streaming();
        let key = self.allocate_key();
        self.entries.push(Entry {
            key,
            message_id,
            role: Role::User,
            text: text.into(),
            images,
            tool: None,
            subagent: None,
            streaming: false,
            time: None,
        });
        Change::Pushed
    }

    /// Records `at` (Unix milliseconds) for newly inserted runtime rows.
    pub fn stamp(&mut self, change: Change, at: i64) {
        let rows = match change {
            Change::Pushed => self.entries.len().saturating_sub(1)..self.entries.len(),
            Change::Spliced {
                start,
                removed: 0,
                inserted,
            } => start..start + inserted,
            _ => return,
        };
        for entry in &mut self.entries[rows] {
            entry.time.get_or_insert(at);
        }
    }

    pub fn apply_runtime(&mut self, thread_id: ThreadId, event: &Event) -> Change {
        match event {
            Event::TurnStarted { .. }
                if !self.snapshot_root_known || self.is_turn_thread(thread_id) =>
            {
                self.turn_thread = Some(thread_id);
                Change::None
            }
            Event::AssistantTextDelta {
                offset,
                message_id,
                text,
                ..
            } if self.is_turn_thread(thread_id) => {
                self.assistant_text(*message_id, text, Some(*offset))
            }
            Event::AssistantMessageCompleted {
                message_id, text, ..
            } if self.is_turn_thread(thread_id) => self.assistant_text(*message_id, text, None),
            Event::AssistantReasoningTextDelta {
                offset,
                message_id,
                text,
            } if self.is_turn_thread(thread_id) => {
                self.reasoning_text(*message_id, Some(*offset), text)
            }
            Event::AssistantReasoningMessageCompleted { message_id, text }
                if self.is_turn_thread(thread_id) =>
            {
                self.reasoning_text(*message_id, None, text)
            }
            Event::SteeringDelivered {
                message_id,
                text,
                images,
                ..
            } => self.push_user(Some(*message_id), text, images.clone()),
            Event::ToolCallRequested { call } => {
                let prepare_tool = || AppTranscriptTool {
                    terminal: None,
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    args: call.args.clone(),
                    effective_args: None,
                    status: "running".to_owned(),
                    result: None,
                    metadata: Value::Null,
                };
                if let Some(index) = self.running_subagent(thread_id) {
                    let subagent = self.entries[index]
                        .subagent
                        .as_mut()
                        .expect("subagent entry");
                    if subagent.tools.len() < MAX_SUBAGENT_TOOLS {
                        let tool = prepare_tool();
                        Arc::make_mut(subagent).tools.push(tool);
                    }
                    return Change::Updated(index);
                }
                let tool = prepare_tool();
                self.stop_streaming();
                let key = self.allocate_key();
                self.entries.push(Entry {
                    key,
                    message_id: None,
                    role: Role::System,
                    text: "".into(),
                    images: Vec::new(),
                    tool: Some(Arc::new(tool)),
                    subagent: None,
                    streaming: false,
                    time: None,
                });
                Change::Pushed
            }
            Event::ToolCallResolved { call } => self.update_tool(&call.id, |tool| {
                tool.effective_args = Some(call.args.clone());
            }),
            Event::ToolTerminalBound { call_id, terminal } => self.update_tool(call_id, |tool| {
                tool.terminal = Some(terminal.clone());
            }),
            Event::ApprovalRequested { call_id, .. } => self.update_tool(call_id, |tool| {
                tool.status = "waiting_approval".to_owned();
            }),
            Event::ApprovalResolved { call_id, approved } => self.update_tool(call_id, |tool| {
                tool.status = if *approved { "approved" } else { "denied" }.to_owned();
            }),
            Event::ToolFinished { result } => {
                self.update_tool(&result.call_id, |tool| finish_tool(tool, result))
            }
            Event::SubagentStarted {
                role,
                description,
                child_thread_id,
            } => {
                self.stop_streaming();
                let key = self.allocate_key();
                self.entries.push(Entry {
                    key,
                    message_id: None,
                    role: Role::System,
                    text: "".into(),
                    images: Vec::new(),
                    tool: None,
                    subagent: Some(Arc::new(AppTranscriptSubagent {
                        child_thread_id: child_thread_id.to_string(),
                        session: None,
                        role: role.clone(),
                        description: description.clone(),
                        status: "running".to_owned(),
                        iterations: None,
                        tools: Vec::new(),
                    })),
                    streaming: false,
                    time: None,
                });
                Change::Pushed
            }
            Event::SubagentSessionBound {
                child_thread_id,
                session_id,
                session_dir,
            } => {
                let child = child_thread_id.to_string();
                let Some(index) = self.entries.iter().rposition(|entry| {
                    entry
                        .subagent
                        .as_ref()
                        .is_some_and(|agent| agent.child_thread_id == child)
                }) else {
                    return Change::None;
                };
                let subagent = self.entries[index].subagent.as_mut().unwrap();
                Arc::make_mut(subagent).session =
                    Some(proteus_contracts::app_protocol::AppSubagentSession {
                        session_id: *session_id,
                        session_dir: session_dir.clone(),
                    });
                Change::Updated(index)
            }
            Event::SubagentFinished {
                status,
                iterations,
                child_thread_id,
                ..
            } => {
                let child = child_thread_id.to_string();
                let Some(index) = self.entries.iter().rposition(|entry| {
                    entry
                        .subagent
                        .as_ref()
                        .is_some_and(|subagent| subagent.child_thread_id == child)
                }) else {
                    return Change::None;
                };
                let subagent = self.entries[index]
                    .subagent
                    .as_mut()
                    .expect("subagent entry");
                let subagent = Arc::make_mut(subagent);
                subagent.status = status.clone();
                subagent.iterations = Some(*iterations);
                Change::Updated(index)
            }
            _ => Change::None,
        }
    }

    fn is_turn_thread(&self, thread_id: ThreadId) -> bool {
        self.turn_thread.is_none_or(|turn| turn == thread_id)
    }

    fn assistant_text(
        &mut self,
        message_id: MessageId,
        text: &str,
        offset: Option<usize>,
    ) -> Change {
        let completed = offset.is_none();
        #[cfg(test)]
        if !completed {
            self.text_work.delta_lookups += 1;
        }
        if let Some(index) = self.entries.iter().rposition(|entry| {
            entry.message_id == Some(message_id) && entry.role == Role::Assistant
        }) {
            let entry = &mut self.entries[index];
            entry.text = match offset {
                Some(offset) => {
                    let keep = delta_keep(&entry.text, offset);
                    #[cfg(test)]
                    {
                        self.text_work.prior_row_prefix_bytes += keep;
                        self.text_work.delta_row_replacements += 1;
                    }
                    splice_delta(&entry.text, keep, text)
                }
                None if *entry.text == *text => entry.text.clone(),
                None => text.into(),
            };
            entry.streaming = !completed;
            return Change::Updated(index);
        }
        if text.is_empty() {
            return Change::None;
        }
        #[cfg(test)]
        if !completed {
            self.text_work.delta_row_replacements += 1;
        }
        self.stop_streaming();
        let key = self.allocate_key();
        self.entries.push(Entry {
            key,
            message_id: Some(message_id),
            role: Role::Assistant,
            text: text.into(),
            images: Vec::new(),
            tool: None,
            subagent: None,
            streaming: !completed,
            time: None,
        });
        Change::Pushed
    }

    fn running_subagent(&self, thread_id: ThreadId) -> Option<usize> {
        let thread = thread_id.to_string();
        self.entries.iter().rposition(|entry| {
            entry.subagent.as_ref().is_some_and(|subagent| {
                subagent.status == "running" && subagent.child_thread_id == thread
            })
        })
    }

    fn update_tool(
        &mut self,
        call_id: &str,
        update: impl FnOnce(&mut AppTranscriptTool),
    ) -> Change {
        for (index, entry) in self.entries.iter_mut().enumerate().rev() {
            if let Some(tool) = entry.tool.as_mut().filter(|tool| tool.call_id == call_id) {
                update(Arc::make_mut(tool));
                return Change::Updated(index);
            }
            if let Some(subagent) = entry.subagent.as_mut()
                && let Some(tool) = subagent.tools.iter().rposition(|tool| tool.call_id == call_id)
            {
                // A reverse lookup must not detach any unrelated progress.
                update(&mut Arc::make_mut(subagent).tools[tool]);
                return Change::Updated(index);
            }
        }
        Change::None
    }

    fn stop_streaming(&mut self) {
        for entry in &mut self.entries {
            entry.streaming = false;
        }
    }

    fn allocate_key(&mut self) -> u64 {
        self.next_key.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn entry_from_with_text(
        &mut self,
        message: AppTranscriptMessage,
        role: Role,
        shared_text: Option<Arc<str>>,
    ) -> Entry {
        Entry::from_wire(self.allocate_key(), message, role, shared_text)
    }
}

impl Entry {
    fn from_wire(
        key: u64,
        message: AppTranscriptMessage,
        role: Role,
        shared_text: Option<Arc<str>>,
    ) -> Self {
        Self {
            key,
            message_id: message.message_id,
            role,
            text: shared_text.unwrap_or_else(|| message.text.into()),
            images: message.images,
            tool: message.tool.map(Arc::new),
            subagent: message.subagent.map(Arc::new),
            streaming: message.streaming,
            time: message.timestamp_ms,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum Identity<'a> {
    Message(MessageId, Role),
    Tool(&'a str),
    Subagent(&'a str),
}

fn role_of(role: &str) -> Role {
    match role {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "reasoning" => Role::Reasoning,
        _ => Role::System,
    }
}

fn identity_of(entry: &Entry) -> Option<Identity<'_>> {
    identity(
        entry.message_id,
        entry.role,
        entry.tool.as_deref(),
        entry.subagent.as_deref(),
    )
}

fn identity<'a>(
    message_id: Option<MessageId>,
    role: Role,
    tool: Option<&'a AppTranscriptTool>,
    subagent: Option<&'a AppTranscriptSubagent>,
) -> Option<Identity<'a>> {
    if let Some(id) = message_id {
        return Some(Identity::Message(id, role));
    }
    if let Some(tool) = tool {
        return Some(Identity::Tool(&tool.call_id));
    }
    subagent.map(|subagent| Identity::Subagent(&subagent.child_thread_id))
}

/// Time of a row pushed locally, before the server gave it an identity:
/// the sent message comes back in the snapshot with its id.
fn local_time_of(old: &mut &[Entry], entry: &Entry) -> Option<i64> {
    // A suffix with no provisional timestamps cannot serve any lookup in this
    // reset. Forget it once, without indexing or hashing message bodies.
    while let Some((last, rest)) = old.split_last() {
        if last.message_id.is_none() && last.time.is_some() {
            break;
        }
        *old = rest;
    }
    old.iter()
        .rev()
        .find(|before| {
            before.message_id.is_none()
                && before.role == entry.role
                && before.text == entry.text
                && before.time.is_some()
        })
        .and_then(|before| before.time)
}

fn delta_keep(current: &str, offset: usize) -> usize {
    if offset <= current.len() && current.is_char_boundary(offset) {
        offset
    } else {
        current.len()
    }
}

fn splice_delta(current: &Arc<str>, keep: usize, delta: &str) -> Arc<str> {
    let mut text = String::with_capacity(keep + delta.len());
    text.push_str(&current[..keep]);
    text.push_str(delta);
    text.into()
}

fn finish_tool(tool: &mut AppTranscriptTool, result: &ToolResult) {
    tool.status = if result.ok { "done" } else { "failed" }.to_owned();
    tool.result = Some(result.text_or_status());
    tool.metadata = result.metadata.clone();
    if let Some(terminal) = result
        .metadata
        .get("tool_terminal")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
    {
        tool.terminal = Some(terminal);
    }
}

/// Whether a row renders the same after a snapshot replaced it.
fn same_row(before: &Entry, after: &Entry) -> bool {
    before.key == after.key
        && before.role == after.role
        && Arc::ptr_eq(&before.text, &after.text)
        && before.time == after.time
        && before.streaming == after.streaming
        && before.images == after.images
        && match (&before.tool, &after.tool) {
            (None, None) => true,
            (Some(a), Some(b)) => same_tool(a, b),
            _ => false,
        }
        && match (&before.subagent, &after.subagent) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                a.status == b.status
                    && a.session == b.session
                    && a.iterations == b.iterations
                    && a.tools.len() == b.tools.len()
                    && a.tools.iter().zip(&b.tools).all(|(a, b)| same_tool(a, b))
            }
            _ => false,
        }
}

fn same_tool(a: &AppTranscriptTool, b: &AppTranscriptTool) -> bool {
    a.call_id == b.call_id
        && a.status == b.status
        && a.result == b.result
        && a.terminal == b.terminal
        && a.effective_args.is_some() == b.effective_args.is_some()
        // Binding can arrive after the tool result without changing its text
        // or status. Compare only the compact reserved fields, not large raw
        // result metadata on every unchanged snapshot row.
        && ["tool", "child_thread_id", "subagent_session"].iter().all(|key| a.metadata.get(*key) == b.metadata.get(*key))
}

mod cache;
mod chains;
mod external;
#[cfg(test)]
mod page_reuse_tests;
mod reasoning;
#[cfg(test)]
mod reasoning_tests;
pub(crate) mod subagent;
mod text_run;

pub(crate) use text_run::{TextRun, TextRunOutcome};

pub use cache::window_changes;

pub(crate) use chains::tool_run;
pub use chains::{affected_rows, tool_chain};

#[cfg(test)]
mod tests;
