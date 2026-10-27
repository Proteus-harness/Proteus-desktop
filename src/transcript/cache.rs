//! Payload accounting and list edits for moving between overlapping windows.
use super::{Change, Entry};
use serde_json::Value;

impl Entry {
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
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
            + self.tool.as_ref().map_or(0, tool_bytes)
            + self.subagent.as_ref().map_or(0, |agent| {
                agent.child_thread_id.len()
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

/// Keep the common rows measured, so GPUI's logical anchor survives eviction.
pub fn window_changes(old: &[Entry], new: &[Entry]) -> Vec<Change> {
    let overlap = old.iter().enumerate().find_map(|(a, row)| {
        new.iter()
            .position(|next| next.key == row.key)
            .map(|b| (a, b))
    });
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
