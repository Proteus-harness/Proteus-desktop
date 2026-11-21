//! One pass over the calls represented by a folded header.
use proteus_contracts::app_protocol::AppTranscriptTool;

use super::super::tool_row::is_running;

pub(super) struct Summary<'a> {
    pub count: usize,
    pub status: &'static str,
    pub labels: Vec<(&'a str, usize)>,
}

pub(super) fn summarize<'a>(tools: impl IntoIterator<Item = &'a AppTranscriptTool>) -> Summary<'a> {
    let mut count = 0;
    let mut status = Status::Done;
    let mut labels: Vec<(&str, usize)> = Vec::new();
    for tool in tools {
        count += 1;
        status = status.max(Status::for_call(&tool.status));
        if let Some((_, count)) = labels.iter_mut().find(|(name, _)| *name == tool.name) {
            *count += 1;
        } else {
            labels.push((&tool.name, 1));
        }
    }
    Summary {
        count,
        status: status.as_str(),
        labels,
    }
}

/// Increasing priority, independent of the order of the calls in the chain.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Done,
    Interrupted,
    Failed,
    WaitingApproval,
    Running,
}

impl Status {
    fn for_call(status: &str) -> Self {
        match status {
            status if is_running(status) => Self::Running,
            "waiting_approval" => Self::WaitingApproval,
            "failed" | "denied" => Self::Failed,
            "done" => Self::Done,
            _ => Self::Interrupted,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::WaitingApproval => "waiting_approval",
            Self::Running => "running",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_labels_keep_first_order_while_approval_uses_highest_priority() {
        let tool = |name: &str, status: &str| AppTranscriptTool {
            call_id: name.into(),
            name: name.into(),
            args: serde_json::Value::Null,
            effective_args: None,
            status: status.into(),
            result: None,
            metadata: serde_json::Value::Null,
            terminal: None,
        };
        let calls = [
            tool("search", "denied"),
            tool("read_file", "unknown"),
            tool("search", "waiting_approval"),
            tool("read_file", "approved"),
        ];
        let waiting = summarize(&calls[..3]);
        assert_eq!(waiting.count, 3);
        assert_eq!(waiting.status, "waiting_approval");
        assert_eq!(waiting.labels, [("search", 2), ("read_file", 1)]);
        let running = summarize(&calls);
        assert_eq!(running.count, 4);
        assert_eq!(running.status, "running");
        assert_eq!(running.labels, [("search", 2), ("read_file", 2)]);
    }
}
