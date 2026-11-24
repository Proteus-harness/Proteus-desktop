//! Payload accounting and list edits for moving between overlapping windows.
use super::{Change, Entry};
use serde_json::Value;

impl Entry {
    pub fn retained_bytes(&self) -> usize {
        // Keep the historical inline charge even when display windows
        // share standalone tool and child-progress payloads.
        use proteus_contracts::app_protocol::{AppTranscriptSubagent, AppTranscriptTool};
        std::mem::size_of::<Entry<AppTranscriptTool, AppTranscriptSubagent>>()
            + self.text.len()
            + self
                .images
                .iter()
                .map(|image| {
                    image.id.len()
                        + image.name.len()
                        + image.mime_type.len()
                        + image.path.as_os_str().len()
                })
                .sum::<usize>()
            + self.tool.as_deref().map_or(0, tool_bytes)
            + self.subagent.as_ref().map_or(0, |agent| {
                agent.child_thread_id.len()
                    + agent
                        .session
                        .as_ref()
                        .map_or(0, |session| session.session_dir.as_os_str().len())
                    + agent.role.len()
                    + agent.description.as_ref().map_or(0, String::len)
                    + agent.status.len()
                    + agent.tools.iter().map(tool_bytes).sum::<usize>()
            })
    }
}

fn tool_bytes(tool: &proteus_contracts::app_protocol::AppTranscriptTool) -> usize {
    std::mem::size_of_val(tool)
        + tool.call_id.len()
        + tool.name.len()
        + tool.status.len()
        + tool.result.as_ref().map_or(0, String::len)
        + value_bytes(&tool.args)
        + tool.effective_args.as_ref().map_or(0, value_bytes)
        + value_bytes(&tool.metadata)
}

fn value_bytes(value: &Value) -> usize {
    std::mem::size_of_val(value)
        + match value {
            Value::String(s) => s.len(),
            Value::Array(items) => items.iter().map(value_bytes).sum(),
            Value::Object(items) => items
                .iter()
                .map(|(key, value)| key.len() + value_bytes(value))
                .sum(),
            _ => 0,
        }
}

/// Keep the first matching old row and its first occurrence in the new view.
fn first_overlap(old: &[Entry], new: &[Entry]) -> Option<(usize, usize)> {
    let first = old.first()?;
    if new.is_empty() {
        return None;
    }
    let position = |row: &Entry| new.iter().position(|next| next.key == row.key);
    if let Some(b) = position(first) {
        return Some((0, b));
    }

    // Nearby overlap is usually cheaper to probe directly. Only switch to
    // binary lookup once there is more work and key order has been verified.
    const DIRECT_CHECKS: usize = 1024;
    const MIN_ORDERED_ROWS: usize = 16;
    let rest = &old[1..];
    let direct = if new.len() >= MIN_ORDERED_ROWS {
        (DIRECT_CHECKS / new.len()).max(4).min(rest.len())
    } else {
        rest.len()
    };
    let (prefix, tail) = rest.split_at(direct);
    if let Some(hit) = prefix
        .iter()
        .enumerate()
        .find_map(|(a, row)| position(row).map(|b| (a + 1, b)))
    {
        return Some(hit);
    }
    if tail.is_empty() {
        return None;
    }

    let offset = direct + 1;
    if new.is_sorted_by_key(|row| row.key) {
        let lowest = new[0].key;
        let highest = new[new.len() - 1].key;
        tail.iter().enumerate().find_map(|(a, row)| {
            if !(lowest..=highest).contains(&row.key) {
                return None;
            }
            // A binary-search hit alone need not be the first duplicate.
            let b = new.partition_point(|next| next.key < row.key);
            (new[b].key == row.key).then_some((a + offset, b))
        })
    } else {
        tail.iter()
            .enumerate()
            .find_map(|(a, row)| position(row).map(|b| (a + offset, b)))
    }
}

/// Keep the common rows measured, so GPUI's logical anchor survives eviction.
pub fn window_changes(old: &[Entry], new: &[Entry]) -> Vec<Change> {
    let overlap = first_overlap(old, new);
    let Some((a, b)) = overlap else {
        return vec![Change::Spliced {
            start: 0,
            removed: old.len(),
            inserted: new.len(),
        }];
    };
    let common = old[a..]
        .iter()
        .zip(&new[b..])
        .take_while(|(a, b)| a.key == b.key)
        .count();
    let mut changes = Vec::new();
    if old.len() > a + common {
        changes.push(Change::Spliced {
            start: a + common,
            removed: old.len() - a - common,
            inserted: 0,
        });
    }
    if a > 0 || b > 0 {
        changes.push(Change::Spliced {
            start: 0,
            removed: a,
            inserted: b,
        });
    }
    if new.len() > b + common {
        changes.push(Change::Spliced {
            start: b + common,
            removed: 0,
            inserted: new.len() - b - common,
        });
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Role;

    fn rows(keys: impl IntoIterator<Item = u64>) -> Vec<Entry> {
        keys.into_iter()
            .map(|key| Entry {
                key,
                message_id: None,
                role: Role::Assistant,
                text: "".into(),
                images: Vec::new(),
                tool: None,
                subagent: None,
                streaming: false,
                time: None,
            })
            .collect()
    }

    #[test]
    fn unordered_overlap_prefers_old_order_then_first_new_duplicate() {
        let old = rows(0..200);
        let mut new = rows(1000..1200);
        new[2].key = 75;
        new[6].key = 65;
        new[30].key = 65;
        new[31].key = 66;
        assert_eq!(first_overlap(&old, &new), Some((65, 6)));
        assert_eq!(
            window_changes(&old, &new),
            [
                Change::Spliced {
                    start: 66,
                    removed: 134,
                    inserted: 0,
                },
                Change::Spliced {
                    start: 0,
                    removed: 65,
                    inserted: 6,
                },
                Change::Spliced {
                    start: 7,
                    removed: 0,
                    inserted: 193,
                },
            ]
        );
    }
    #[test]
    fn ordered_overlap_chooses_the_first_duplicate() {
        let old = rows(0..200);
        let new = rows([65, 65, 66].into_iter().chain(1000..1197));
        assert_eq!(first_overlap(&old, &new), Some((65, 0)));
    }
}
