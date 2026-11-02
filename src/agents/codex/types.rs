use serde_json::Value;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page<T> {
    pub data: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub id: String,
    pub title: String,
    pub cwd: PathBuf,
    pub parent_thread_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: String,
    pub model: String,
    pub title: String,
    pub description: String,
    pub default: bool,
    pub default_effort: String,
    pub efforts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub turn_id: String,
    pub kind: ItemKind,
    pub status: String,
    pub timestamp_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ItemKind {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    Reasoning {
        text: String,
    },
    Tool {
        name: String,
        input: Value,
        output: String,
    },
    Plan {
        text: String,
    },
    Other {
        name: String,
        text: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CatalogEvent {
    /// Authoritative server notification; absence from a catalog page is not deletion.
    ThreadDeleted { id: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    ItemUpsert(Item),
    TextDelta {
        id: String,
        turn_id: String,
        delta: String,
    },
    ReasoningDelta {
        id: String,
        turn_id: String,
        delta: String,
    },
    ToolOutputDelta {
        id: String,
        turn_id: String,
        delta: String,
    },
    TurnStarted {
        id: String,
    },
    TurnCompleted {
        id: String,
        status: String,
        error: Option<String>,
    },
    PendingRequest {
        id: Value,
        method: String,
        params: Value,
    },
    RequestResolved {
        id: Value,
    },
    Plan(Value),
    Usage(Value),
    /// Temporarily disable input without discarding the draft or interactions.
    SubscriptionSuspended,
    /// Clear interactions from the old subscription before new ones can arrive.
    ResyncStarted,
    /// Subscription restored after a definitively rejected deletion. History is
    /// a bounded tail, excluding items changed by live events during recovery.
    Resynced {
        current_turn: Option<String>,
        history: Vec<Item>,
    },
    Disconnected(String),
}
