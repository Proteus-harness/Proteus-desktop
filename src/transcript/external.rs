//! Opaque agent items fold directly into display rows, without Core events or ids.
use super::{Change, Entry, Role, Transcript};
use crate::agents::codex::{Item, ItemKind};
use proteus_contracts::app_protocol::AppTranscriptTool;
use serde_json::Value;

impl Transcript {
    pub(crate) fn external_upsert(&mut self, item: Item) -> Change {
        let identity = (item.turn_id, item.id);
        let streaming = matches!(item.status.as_str(), "inProgress" | "running");
        let (role, text, tool) = match item.kind {
            ItemKind::User { text } => (Role::User, text, None),
            ItemKind::Assistant { text } | ItemKind::Plan { text } => (Role::Assistant, text, None),
            ItemKind::Reasoning { text } | ItemKind::Other { text, .. } => {
                (Role::System, text, None)
            }
            ItemKind::Tool {
                name,
                input,
                output,
            } => (
                Role::System,
                String::new(),
                Some(AppTranscriptTool {
                    terminal: None,
                    call_id: identity.1.clone(),
                    name,
                    args: input,
                    effective_args: None,
                    status: if streaming {
                        "running"
                    } else if matches!(item.status.as_str(), "failed" | "declined") {
                        "failed"
                    } else {
                        "done"
                    }
                    .into(),
                    result: (!output.is_empty()).then_some(output),
                    metadata: Value::Null,
                }),
            ),
        };
        if let Some(index) = self.external_index(&identity) {
            let row = &mut self.entries[index];
            if !row.streaming && streaming {
                return Change::None;
            }
            let unchanged = row.role == role
                && row.text.as_ref() == text
                && row.streaming == streaming
                && match (&row.tool, &tool) {
                    (None, None) => true,
                    (Some(a), Some(b)) => {
                        a.name == b.name
                            && a.args == b.args
                            && a.result == b.result
                            && a.status == b.status
                    }
                    _ => false,
                }
                && (item.timestamp_ms.is_none() || row.time == item.timestamp_ms);
            if unchanged {
                return Change::None;
            }
            if row.text.as_ref() != text {
                row.text = text.into();
            }
            row.role = role;
            row.tool = tool;
            row.streaming = streaming;
            row.time = item.timestamp_ms.or(row.time);
            return Change::Updated(index);
        }
        let key = self.allocate_key();
        self.external_ids.insert(identity, key);
        self.entries.push(Entry {
            key,
            message_id: None,
            role,
            text: text.into(),
            images: vec![],
            tool,
            subagent: None,
            streaming,
            time: item.timestamp_ms,
        });
        Change::Pushed
    }

    pub(crate) fn external_delta(
        &mut self,
        turn: &str,
        id: &str,
        delta: &str,
        reasoning: bool,
        tool: bool,
    ) -> Change {
        let identity = (turn.to_owned(), id.to_owned());
        let Some(index) = self.external_index(&identity) else {
            return self.external_upsert(Item {
                id: id.into(),
                turn_id: turn.into(),
                status: "inProgress".into(),
                timestamp_ms: None,
                kind: if tool {
                    ItemKind::Tool {
                        name: "Инструмент".into(),
                        input: Value::Null,
                        output: delta.into(),
                    }
                } else if reasoning {
                    ItemKind::Reasoning { text: delta.into() }
                } else {
                    ItemKind::Assistant { text: delta.into() }
                },
            });
        };
        let entry = &mut self.entries[index];
        // A delayed queued delta cannot reopen an already completed canonical item.
        if !entry.streaming || delta.is_empty() {
            return Change::None;
        }
        const MAX_TEXT: usize = 128 * 1024;
        let current = if tool {
            entry
                .tool
                .as_ref()
                .and_then(|tool| tool.result.as_ref())
                .map_or(0, String::len)
        } else {
            entry.text.len()
        };
        let mut end = delta.len().min(MAX_TEXT.saturating_sub(current));
        while !delta.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            return Change::None;
        }
        if tool {
            if let Some(tool) = &mut entry.tool {
                tool.result.get_or_insert_default().push_str(&delta[..end]);
            }
        } else {
            let mut text = String::with_capacity(entry.text.len() + end);
            text.push_str(&entry.text);
            text.push_str(&delta[..end]);
            entry.text = text.into();
        }
        Change::Updated(index)
    }

    pub(crate) fn external_reidentify(
        &mut self,
        old_turn: &str,
        old_id: &str,
        turn: &str,
        id: &str,
    ) {
        if let Some(key) = self.external_ids.remove(&(old_turn.into(), old_id.into())) {
            self.external_ids.insert((turn.into(), id.into()), key);
        }
    }

    pub(crate) fn external_remove(&mut self, turn: &str, id: &str) -> Change {
        let identity = (turn.into(), id.into());
        let Some(index) = self.external_index(&identity) else {
            return Change::None;
        };
        self.external_ids.remove(&identity);
        self.remove(index..index + 1)
    }

    pub(crate) fn external_contains(&self, turn: &str, id: &str) -> bool {
        self.external_index(&(turn.into(), id.into())).is_some()
    }

    pub(crate) fn external_first_identity(&self) -> Option<(String, String)> {
        let first = self.entries.first()?.key;
        self.external_ids
            .iter()
            .find(|(_, key)| **key == first)
            .map(|(identity, _)| identity.clone())
    }

    pub(crate) fn external_identity(&self, key: u64) -> Option<(&str, &str)> {
        self.external_ids
            .iter()
            .find(|(_, entry)| **entry == key)
            .map(|((turn, item), _)| (turn.as_str(), item.as_str()))
    }

    pub(crate) fn external_trim_budget(&mut self, limit: usize, bytes: usize) -> Change {
        let mut count = self.entries.len().saturating_sub(limit);
        let mut retained = self.entries[count..]
            .iter()
            .map(Entry::retained_bytes)
            .sum::<usize>();
        while retained > bytes && count + 1 < self.entries.len() {
            retained = retained.saturating_sub(self.entries[count].retained_bytes());
            count += 1;
        }
        if count == 0 {
            return Change::None;
        }
        let change = self.remove(0..count);
        self.external_prune();
        change
    }

    pub(crate) fn external_prune(&mut self) {
        let keys: std::collections::HashSet<_> = self.entries.iter().map(|row| row.key).collect();
        self.external_ids.retain(|_, key| keys.contains(key));
    }

    fn external_index(&self, identity: &(String, String)) -> Option<usize> {
        let key = self.external_ids.get(identity)?;
        self.entries.iter().position(|row| row.key == *key)
    }

    pub(crate) fn external_prepend(&mut self, items: Vec<Item>) -> Change {
        let old_count = self.entries.len();
        let mut prefix = Transcript {
            next_key: self.next_key.clone(),
            ..Default::default()
        };
        for item in items {
            let identity = (item.turn_id.clone(), item.id.clone());
            if self.external_index(&identity).is_none() {
                prefix.external_upsert(item);
            }
        }
        let inserted = prefix.entries.len();
        prefix.entries.append(&mut self.entries);
        self.entries = prefix.entries;
        self.external_ids.extend(prefix.external_ids);
        debug_assert_eq!(self.entries.len(), old_count + inserted);
        Change::Spliced {
            start: 0,
            removed: 0,
            inserted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: &str, turn: &str, text: &str, done: bool) -> Item {
        Item {
            id: id.into(),
            turn_id: turn.into(),
            kind: ItemKind::Assistant { text: text.into() },
            status: if done { "completed" } else { "inProgress" }.into(),
            timestamp_ms: None,
        }
    }
    #[::core::prelude::v1::test]
    fn opaque_identity_replay_and_final_delta_are_stable() {
        let mut transcript = Transcript::default();
        transcript.external_upsert(item("item:opaque", "turn/opaque", "a", false));
        let key = transcript.entries()[0].key;
        transcript.external_delta("turn/opaque", "item:opaque", "b", false, false);
        transcript.external_upsert(item("item:opaque", "turn/opaque", "ab", true));
        assert_eq!(
            transcript.external_upsert(item("item:opaque", "turn/opaque", "ab", true)),
            Change::None
        );
        assert_eq!(
            transcript.external_delta("turn/opaque", "item:opaque", "stale", false, false),
            Change::None
        );
        assert_eq!(
            transcript.external_upsert(item("item:opaque", "turn/opaque", "", false)),
            Change::None
        );
        assert_eq!(transcript.entries()[0].key, key);
        assert_eq!(transcript.entries()[0].text.as_ref(), "ab");
        transcript.external_upsert(item("item:opaque", "different-turn", "new", true));
        assert_eq!(transcript.entries().len(), 2);
    }
    #[::core::prelude::v1::test]
    fn streaming_retention_is_utf8_bounded_and_unchanged_rows_share_text() {
        let mut transcript = Transcript::default();
        transcript.external_upsert(item("a", "t", "", false));
        transcript.external_delta("t", "a", &"я".repeat(100_000), false, false);
        assert_eq!(transcript.entries()[0].text.len(), 128 * 1024);
        assert_eq!(
            transcript.external_delta("t", "a", "ещё", false, false),
            Change::None
        );
        let allocation = transcript.entries()[0].text.clone();
        let text = allocation.to_string();
        transcript.external_upsert(item("a", "t", &text, true));
        assert!(std::sync::Arc::ptr_eq(
            &allocation,
            &transcript.entries()[0].text
        ));
    }
    #[::core::prelude::v1::test]
    fn paging_deduplicates_and_reconciles_optimistic_key() {
        let mut transcript = Transcript::default();
        transcript.external_upsert(item("local", "", "hello", true));
        let key = transcript.entries()[0].key;
        transcript.external_reidentify("", "local", "turn", "server");
        transcript.external_upsert(item("server", "turn", "hello", true));
        transcript.external_prepend(vec![
            item("older", "t", "old", true),
            item("server", "turn", "hello", true),
        ]);
        assert_eq!(transcript.entries().len(), 2);
        assert_eq!(transcript.entries()[1].key, key);
        transcript.external_trim_budget(1, usize::MAX);
        assert_eq!(transcript.external_ids.len(), 1);
    }
    #[::core::prelude::v1::test]
    fn paused_reader_snapshot_survives_live_eviction_with_identical_row_keys() {
        let mut live = Transcript::default();
        for index in 0..50 {
            live.external_upsert(item(
                &index.to_string(),
                "turn",
                &format!("row {index}"),
                true,
            ));
        }
        let paused = live.clone();
        let keys: Vec<_> = paused.entries().iter().map(|row| row.key).collect();
        for index in 50..500 {
            live.external_upsert(item(&index.to_string(), "turn", "new", true));
            live.external_trim_budget(50, 8 * 1024 * 1024);
        }
        assert_eq!(
            paused
                .entries()
                .iter()
                .map(|row| row.key)
                .collect::<Vec<_>>(),
            keys
        );
        assert_eq!(paused.entries()[0].text.as_ref(), "row 0");
        assert_eq!(live.entries().len(), 50);
        assert_eq!(live.external_ids.len(), 50);
    }
    #[::core::prelude::v1::test]
    fn live_tail_obeys_byte_budget_as_well_as_row_limit() {
        let mut live = Transcript::default();
        for index in 0..4 {
            live.external_upsert(item(
                &index.to_string(),
                "turn",
                &"x".repeat(128 * 1024),
                true,
            ));
        }
        live.external_trim_budget(50, 200 * 1024);
        assert_eq!(live.entries().len(), 1);
        assert!(
            live.entries()
                .iter()
                .map(Entry::retained_bytes)
                .sum::<usize>()
                <= 200 * 1024
        );
    }
}
