//! Runs of consecutive tool calls that the chat may fold into one row.

use std::ops::Range;

use super::{Entry, Role};

/// Shorter runs stay expanded: folding two calls saves nothing.
pub const MIN_CHAIN: usize = 3;

fn is_tool(entry: &Entry) -> bool {
    entry.role == Role::System
        && entry
            .tool
            .as_ref()
            .is_some_and(|tool| !super::subagent::is_task(tool))
}

/// The run of tool rows containing `ix`, when it is long enough to fold.
pub fn tool_chain(entries: &[Entry], ix: usize) -> Option<Range<usize>> {
    tool_run(entries, ix).filter(|run| run.len() >= MIN_CHAIN)
}

/// Rows whose look may change when row `ix` changes: the whole run of tool
/// rows around it, since a growing run can start folding.
pub fn affected_rows(entries: &[Entry], ix: usize) -> Range<usize> {
    let mut rows = tool_run(entries, ix).unwrap_or(ix..(ix + 1).min(entries.len()));
    if let Some(entry) = entries.get(ix) {
        if entry.tool.as_ref().is_some_and(super::subagent::is_task) {
            // A newly bound call may have split a former generic tool chain.
            if let Some(previous) = ix.checked_sub(1).and_then(|row| tool_run(entries, row)) {
                rows.start = previous.start;
            }
            if let Some(next) = tool_run(entries, ix + 1) {
                rows.end = next.end;
            }
        }
        let peer = if let Some(tool) = &entry.tool {
            super::subagent::progress_row(entries, tool)
        } else if entry.subagent.is_some() {
            entries.iter().position(|entry| {
                entry
                    .tool
                    .as_ref()
                    .is_some_and(|tool| super::subagent::progress_row(entries, tool) == Some(ix))
            })
        } else {
            None
        };
        if let Some(peer) = peer {
            rows.start = rows.start.min(peer);
            rows.end = rows.end.max(peer + 1);
        }
    }
    rows
}

fn tool_run(entries: &[Entry], ix: usize) -> Option<Range<usize>> {
    if !entries.get(ix).is_some_and(is_tool) {
        return None;
    }
    let start = entries[..ix]
        .iter()
        .rposition(|entry| !is_tool(entry))
        .map_or(0, |before| before + 1);
    let end = entries[ix..]
        .iter()
        .position(|entry| !is_tool(entry))
        .map_or(entries.len(), |after| ix + after);
    Some(start..end)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use proteus_contracts::app_protocol::AppTranscriptTool;

    use super::*;

    fn entry(key: u64, tool: bool) -> Entry {
        let tool = tool.then(|| AppTranscriptTool {
            call_id: format!("c{key}"),
            name: "read_file".into(),
            args: serde_json::Value::Null,
            effective_args: None,
            status: "done".into(),
            result: None,
            metadata: serde_json::Value::Null,
        });
        Entry {
            key,
            message_id: None,
            role: if tool.is_some() {
                Role::System
            } else {
                Role::Assistant
            },
            text: Arc::from(""),
            images: Vec::new(),
            tool,
            subagent: None,
            streaming: false,
            time: None,
        }
    }

    #[test]
    fn folds_only_long_runs_of_tools() {
        let rows = [
            entry(0, false),
            entry(1, true),
            entry(2, true),
            entry(3, false),
            entry(4, true),
            entry(5, true),
            entry(6, true),
        ];
        assert_eq!(tool_chain(&rows, 0), None);
        assert_eq!(tool_chain(&rows, 1), None);
        assert_eq!(tool_chain(&rows, 5), Some(4..7));
        assert_eq!(affected_rows(&rows, 6), 4..7);
        assert_eq!(affected_rows(&rows, 3), 3..4);
    }

    #[test]
    fn bound_child_strips_break_generic_tool_chains_and_invalidate_old_neighbors() {
        let mut rows = vec![
            entry(0, true),
            entry(1, true),
            entry(2, true),
            entry(3, true),
            entry(4, true),
        ];
        assert_eq!(tool_chain(&rows, 2), Some(0..5));
        rows[2].tool.as_mut().unwrap().metadata =
            serde_json::json!({"tool":"task","child_thread_id":"exact-child"});
        assert_eq!(tool_chain(&rows, 2), None);
        assert_eq!(tool_chain(&rows, 0), None);
        assert_eq!(affected_rows(&rows, 2), 0..5);
        let bound = proteus_contracts::app_protocol::AppSubagentSession {
            session_id: uuid::Uuid::from_u128(5),
            session_dir: "/saved/child".into(),
        };
        rows[2].tool.as_mut().unwrap().metadata["subagent_session"] =
            serde_json::to_value(&bound).unwrap();
        let mut progress = entry(5, false);
        progress.role = Role::System;
        progress.subagent = Some(proteus_contracts::app_protocol::AppTranscriptSubagent {
            child_thread_id: "exact-child".into(),
            session: Some(bound),
            role: "explorer".into(),
            description: None,
            status: "running".into(),
            iterations: None,
            tools: Vec::new(),
        });
        rows.push(progress);
        assert_eq!(
            super::super::subagent::progress_row(&rows, rows[2].tool.as_ref().unwrap()),
            Some(5)
        );
        assert_eq!(affected_rows(&rows, 2), 0..6);
        assert_eq!(affected_rows(&rows, 5), 2..6);
        rows[5]
            .subagent
            .as_mut()
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .session_dir = "/conflict".into();
        assert_eq!(
            super::super::subagent::progress_row(&rows, rows[2].tool.as_ref().unwrap()),
            None
        );
    }
    /// The preparation walk asks once per group, not once per tool member.
    fn preparation_groups(rows: &[Entry], compact: bool) -> Vec<Range<usize>> {
        let mut groups = Vec::new();
        let mut row = 0;
        while row < rows.len() {
            let group = compact
                .then(|| tool_chain(rows, row))
                .flatten()
                .unwrap_or(row..row + 1);
            row = group.end;
            groups.push(group);
        }
        groups
    }

    #[test]
    fn forward_groups_keep_short_runs_and_separators_independent() {
        let rows: Vec<_> = [false, true, true, false, true, true, true, false, true]
            .into_iter()
            .enumerate()
            .map(|(key, tool)| entry(key as u64, tool))
            .collect();
        assert_eq!(
            preparation_groups(&rows, true),
            vec![0..1, 1..2, 2..3, 3..4, 4..7, 7..8, 8..9]
        );
        assert_eq!(
            preparation_groups(&rows, false),
            (0..rows.len()).map(|row| row..row + 1).collect::<Vec<_>>()
        );
        assert!(preparation_groups(&[], true).is_empty());
    }

    #[test]
    fn forward_groups_visit_a_long_chain_once_and_recompute_after_growth() {
        let mut rows = vec![entry(0, true), entry(1, true)];
        assert_eq!(preparation_groups(&rows, true), vec![0..1, 1..2]);
        rows.push(entry(2, true));
        assert_eq!(preparation_groups(&rows, true), vec![0..3]);
        rows.extend((3..4096).map(|key| entry(key, true)));
        assert_eq!(preparation_groups(&rows, true), vec![0..4096]);
        rows[2048] = entry(2048, false);
        assert_eq!(
            preparation_groups(&rows, true),
            vec![0..2048, 2048..2049, 2049..4096]
        );
    }
}
