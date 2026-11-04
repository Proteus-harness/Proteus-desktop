//! Summary-only conversations shared by driver-specific transports and UI.
//! A Codex thread is never represented as a fabricated Core directory or UUID.

use std::collections::HashMap;
use std::path::PathBuf;

use proteus_contracts::app_protocol::{AppSessionActivity, AppSessionSummary};
use proteus_tabs::ChatKey;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    #[default]
    Proteus,
    Codex,
}

impl AgentKind {
    pub fn for_key(key: &ChatKey) -> Self {
        match key {
            ChatKey::Proteus(_) => Self::Proteus,
            ChatKey::Codex(_) => Self::Codex,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Proteus => "Proteus",
            Self::Codex => "Codex",
        }
    }
}

pub fn from_codex(summary: &crate::agents::codex::Summary) -> ConversationSummary {
    ConversationSummary {
        key: ChatKey::Codex(summary.id.clone()),
        proteus_session_id: None,
        parent: summary.parent_thread_id.clone().map(ChatKey::Codex),
        workspace_path: summary.cwd.clone(),
        message_count: 0,
        updated_at_ms: u64::try_from(summary.updated_at)
            .ok()
            .map(|value| value.saturating_mul(1000)),
        preview: (!summary.title.is_empty()).then(|| summary.title.clone()),
        activity: None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationSummary {
    pub key: ChatKey,
    /// Durable server identity, retained without copying the directory in `key`.
    pub proteus_session_id: Option<uuid::Uuid>,
    pub parent: Option<ChatKey>,
    pub workspace_path: PathBuf,
    pub message_count: usize,
    pub updated_at_ms: Option<u64>,
    pub preview: Option<String>,
    pub activity: Option<AppSessionActivity>,
}

/// Refresh metadata in place. Merely opening an existing conversation must
/// not move its sidebar row; newly discovered conversations append normally.
pub fn upsert_summary(summaries: &mut Vec<ConversationSummary>, summary: ConversationSummary) {
    if let Some(existing) = summaries.iter_mut().find(|row| row.key == summary.key) {
        *existing = summary;
    } else {
        summaries.push(summary);
    }
}

/// Resolve parent UUIDs while the driver's complete summary catalog is present.
/// Missing parents remain accessible roots; no transcript is needed.
pub fn from_proteus(sessions: Vec<AppSessionSummary>) -> Vec<ConversationSummary> {
    let keys: HashMap<_, _> = sessions
        .iter()
        .map(|summary| {
            (
                summary.session_id,
                ChatKey::Proteus(summary.session_dir.clone()),
            )
        })
        .collect();
    sessions
        .into_iter()
        .map(|summary| ConversationSummary {
            key: ChatKey::Proteus(summary.session_dir),
            proteus_session_id: Some(summary.session_id),
            parent: summary
                .parent_session_id
                .and_then(|id| keys.get(&id).cloned()),
            workspace_path: summary.workspace_path,
            message_count: summary.message_count,
            updated_at_ms: summary.updated_at_ms,
            preview: summary.preview,
            activity: summary.activity,
        })
        .collect()
}

/// Explicit native Codex choices, kept separate from server configuration.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodexSelection {
    pub model: String,
    pub effort: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CodexSelections {
    pub last: Option<CodexSelection>,
    pub threads: std::collections::BTreeMap<String, CodexSelection>,
    pub recent: Vec<String>,
}

impl CodexSelections {
    pub fn remember(&mut self, thread: &str, selection: CodexSelection) {
        self.last = Some(selection.clone());
        self.threads.insert(thread.to_owned(), selection);
        self.recent.retain(|id| id != thread);
        self.recent.push(thread.to_owned());
        while self.recent.len() > 256 {
            let id = self.recent.remove(0);
            self.threads.remove(&id);
        }
    }

    pub fn selection(
        &self,
        thread: &str,
        is_new: bool,
        models: &[crate::agents::codex::Model],
    ) -> Option<CodexSelection> {
        let saved = self
            .threads
            .get(thread)
            .or_else(|| is_new.then_some(self.last.as_ref()).flatten())?;
        let model = models.iter().find(|model| model.model == saved.model)?;
        Some(CodexSelection {
            model: model.model.clone(),
            effort: saved
                .effort
                .as_ref()
                .filter(|effort| model.efforts.contains(effort))
                .cloned()
                .or_else(|| {
                    model
                        .efforts
                        .contains(&model.default_effort)
                        .then(|| model.default_effort.clone())
                }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proteus_contracts::domain::SessionId;

    #[test]
    fn opening_and_refreshing_existing_summaries_preserve_sidebar_positions() {
        let summary = |id: &str| ConversationSummary {
            key: ChatKey::Codex(id.into()),
            proteus_session_id: None,
            parent: None,
            workspace_path: "/workspace".into(),
            message_count: 0,
            updated_at_ms: Some(123),
            preview: Some(id.into()),
            activity: None,
        };
        let mut rows: Vec<_> = ["home", "a", "b", "c"].into_iter().map(summary).collect();
        let expected: Vec<_> = rows.iter().map(|row| row.key.clone()).collect();
        for id in ["b", "a", "c", "b", "a", "home"] {
            let mut updated = summary(id);
            updated.preview = Some(format!("Updated {id}"));
            updated.message_count = 42;
            updated.parent = Some(ChatKey::Codex("parent".into()));
            upsert_summary(&mut rows, updated.clone());
            assert_eq!(
                rows.iter().map(|row| row.key.clone()).collect::<Vec<_>>(),
                expected
            );
            assert_eq!(
                rows.iter().find(|row| row.key == updated.key),
                Some(&updated)
            );
        }
        upsert_summary(&mut rows, summary("new"));
        assert_eq!(rows.len(), expected.len() + 1);
        assert_eq!(rows.last().unwrap().key, ChatKey::Codex("new".into()));
    }

    #[test]
    fn parent_uuid_is_resolved_to_the_real_driver_key_and_missing_parent_stays_accessible() {
        let summary = |id: u128, parent: Option<u128>| {
            let mut summary = AppSessionSummary::new(
                format!("/sessions/{id}").into(),
                SessionId::from_u128(id),
                "/workspace".into(),
                3,
                Some(12),
                Some("title".into()),
            );
            summary.parent_session_id = parent.map(SessionId::from_u128);
            summary
        };
        let rows = from_proteus(vec![
            summary(2, Some(1)),
            summary(1, None),
            summary(3, Some(9)),
        ]);
        assert_eq!(rows[0].parent, Some(rows[1].key.clone()));
        assert_eq!(rows[0].proteus_session_id, Some(SessionId::from_u128(2)));
        assert_eq!(rows[2].parent, None);
        assert_eq!(rows[0].message_count, 3);
        assert_eq!(rows[0].workspace_path, PathBuf::from("/workspace"));
    }
    fn models() -> Vec<crate::agents::codex::Model> {
        ["server-model", "chosen-model"]
            .into_iter()
            .map(|name| crate::agents::codex::Model {
                id: format!("catalog:{name}"),
                model: name.into(),
                title: name.into(),
                description: String::new(),
                default: name == "server-model",
                default_effort: "medium".into(),
                efforts: vec!["low".into(), "medium".into(), "high".into()],
            })
            .collect()
    }
    #[test]
    fn explicit_codex_selection_survives_restart_for_current_and_new_threads() {
        let mut preferences = CodexSelections::default();
        assert!(preferences.selection("empty", true, &models()).is_none());
        let first = CodexSelection {
            model: "chosen-model".into(),
            effort: Some("high".into()),
        };
        preferences.remember("opaque:first", first.clone());
        preferences.remember(
            "opaque:second",
            CodexSelection {
                model: "server-model".into(),
                effort: Some("low".into()),
            },
        );
        let restored: CodexSelections =
            serde_json::from_value(serde_json::to_value(preferences).unwrap()).unwrap();
        assert_eq!(
            restored.selection("opaque:first", false, &models()),
            Some(first)
        );
        assert_eq!(
            restored.selection("new", true, &models()).unwrap().model,
            "server-model"
        );
        assert!(
            restored
                .selection("unselected-existing", false, &models())
                .is_none()
        );
    }
    #[test]
    fn removed_models_are_not_sent_and_unsupported_effort_uses_model_default() {
        let mut preferences = CodexSelections::default();
        preferences.remember(
            "thread",
            CodexSelection {
                model: "removed".into(),
                effort: Some("high".into()),
            },
        );
        assert!(preferences.selection("thread", false, &models()).is_none());
        preferences.remember(
            "thread",
            CodexSelection {
                model: "chosen-model".into(),
                effort: Some("unsupported".into()),
            },
        );
        assert_eq!(
            preferences
                .selection("thread", false, &models())
                .unwrap()
                .effort
                .as_deref(),
            Some("medium")
        );
    }
    #[test]
    fn explicit_codex_thread_preferences_have_a_bounded_recent_cache() {
        let mut preferences = CodexSelections::default();
        for index in 0..300 {
            preferences.remember(
                &format!("thread:{index}"),
                CodexSelection {
                    model: "chosen-model".into(),
                    effort: None,
                },
            );
        }
        assert_eq!(preferences.threads.len(), 256);
        assert_eq!(preferences.recent.len(), 256);
        assert!(!preferences.threads.contains_key("thread:0"));
        assert!(preferences.threads.contains_key("thread:299"));
    }
}
