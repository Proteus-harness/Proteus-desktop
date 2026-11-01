//! A conversation's native identity, independent of its owning driver.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ChatKey {
    Proteus(PathBuf),
    Codex(String),
}

impl ChatKey {
    pub fn tab_id(&self) -> String {
        match self {
            Self::Proteus(path) => super::chat(path),
            Self::Codex(id) => codex(id),
        }
    }

    /// Only Proteus conversations have a local session-directory identity.
    pub fn proteus_dir(&self) -> Option<&Path> {
        match self {
            Self::Proteus(path) => Some(path),
            Self::Codex(_) => None,
        }
    }
}

impl fmt::Display for ChatKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tab_id())
    }
}

pub fn codex(thread_id: &str) -> String {
    format!("codex:{thread_id}")
}

/// Preserve opaque driver ids, including any additional colon separators.
pub fn chat_key(id: &str) -> Option<ChatKey> {
    if let Some(path) = super::chat_dir(id) {
        Some(ChatKey::Proteus(path.to_path_buf()))
    } else {
        id.strip_prefix("codex:")
            .map(|id| ChatKey::Codex(id.to_owned()))
    }
}
