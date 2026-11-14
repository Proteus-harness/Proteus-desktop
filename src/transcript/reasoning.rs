//! Readable reasoning uses its own append-only stream, never answer splice rules.
use super::{Change, Entry, MessageId, Role, Transcript};

impl Transcript {
    pub(super) fn reasoning_text(
        &mut self,
        message_id: MessageId,
        offset: Option<usize>,
        text: &str,
    ) -> Change {
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.message_id == Some(message_id) && entry.role == Role::Reasoning)
        {
            if offset.is_none() && text.trim().is_empty() {
                return self.remove(index..index + 1);
            }
            let entry = &mut self.entries[index];
            let updated = if let Some(offset) = offset {
                if offset > entry.text.len() || !entry.text.is_char_boundary(offset) {
                    return Change::None;
                }
                let overlap = (entry.text.len() - offset).min(text.len());
                if !text.is_char_boundary(overlap)
                    || !entry.text.is_char_boundary(offset + overlap)
                    || entry.text[offset..offset + overlap] != text[..overlap]
                {
                    return Change::None;
                }
                let missing = &text[overlap..];
                if missing.is_empty() {
                    false
                } else {
                    let mut value = String::with_capacity(entry.text.len() + missing.len());
                    value.push_str(&entry.text);
                    value.push_str(missing);
                    entry.text = value.into();
                    true
                }
            } else if *entry.text == *text {
                false
            } else {
                entry.text = text.into();
                true
            };
            let streaming = offset.is_some();
            if !updated && entry.streaming == streaming {
                return Change::None;
            }
            entry.streaming = streaming;
            return Change::Updated(index);
        }
        if text.is_empty()
            || offset.is_some_and(|offset| offset != 0)
            || offset.is_none() && text.trim().is_empty()
        {
            return Change::None;
        }
        self.stop_streaming();
        // A provider may disclose reasoning only in the authoritative response.
        // Its display order still precedes the answer sharing this item id.
        let index = self
            .entries
            .iter()
            .position(|entry| entry.message_id == Some(message_id) && entry.role == Role::Assistant)
            .unwrap_or(self.entries.len());
        let tail = index == self.entries.len();
        let key = self.allocate_key();
        self.entries.insert(
            index,
            Entry {
                key,
                message_id: Some(message_id),
                role: Role::Reasoning,
                text: text.into(),
                images: Vec::new(),
                tool: None,
                subagent: None,
                streaming: offset.is_some(),
                time: None,
            },
        );
        if tail {
            Change::Pushed
        } else {
            Change::Spliced {
                start: index,
                removed: 0,
                inserted: 1,
            }
        }
    }
}
