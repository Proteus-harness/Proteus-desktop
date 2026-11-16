//! Window-owned selection intent; late session loads cannot undo a manual pick.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::{Registry, select_path};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionOrigin {
    #[default]
    Default,
    Binding,
    Manual,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowSelection {
    /// None selects the existing built-in/default Desktop visual values.
    pub path: Option<PathBuf>,
    pub origin: SelectionOrigin,
}

impl WindowSelection {
    pub fn select_manual(&mut self, path: &Path, agent: Option<&Path>) -> Result<()> {
        self.path = Some(select_path(path, agent)?);
        self.origin = SelectionOrigin::Manual;
        Ok(())
    }

    /// Returns whether the automatic source changed. The caller only loads a
    /// newly selected source, so ordinary model/session events do not reload it.
    pub fn apply_agent_binding(
        &mut self,
        registry: &Registry,
        agent: Option<&Path>,
    ) -> Result<bool> {
        if self.origin == SelectionOrigin::Manual {
            return Ok(false);
        }
        let selected = registry.resolve(agent)?;
        let changed = *self != selected;
        *self = selected;
        Ok(changed)
    }

    pub fn reset_to_auto(&mut self, registry: &Registry, agent: Option<&Path>) -> Result<()> {
        *self = registry.resolve(agent)?;
        Ok(())
    }
}
