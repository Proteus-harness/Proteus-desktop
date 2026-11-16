//! App-owned associations; neither side is written while binding files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use super::{SelectionOrigin, WindowSelection, select_path};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Registry {
    pub default_config: Option<PathBuf>,
    pub bindings: BTreeMap<PathBuf, PathBuf>,
}

impl Registry {
    pub fn bind(&mut self, agent: &Path, visual: &Path) -> Result<()> {
        let agent = canonical_agent(agent)?;
        let visual = select_path(visual, Some(&agent))?;
        super::load(&visual, Some(&agent))?;
        self.bindings.insert(agent, visual);
        Ok(())
    }

    pub fn unbind(&mut self, agent: &Path) -> Result<Option<PathBuf>> {
        Ok(self.bindings.remove(&canonical_agent(agent)?))
    }

    pub fn set_default(&mut self, path: Option<&Path>) -> Result<()> {
        let selected = path.map(|path| select_path(path, None)).transpose()?;
        if let Some(path) = selected.as_deref() {
            super::load(path, None)?;
        }
        self.default_config = selected;
        Ok(())
    }

    pub fn resolve(&self, agent: Option<&Path>) -> Result<WindowSelection> {
        if let Some(agent) = agent {
            let agent = canonical_agent(agent)?;
            if let Some(path) = self.bindings.get(&agent) {
                return Ok(WindowSelection {
                    path: Some(select_path(path, Some(&agent))?),
                    origin: SelectionOrigin::Binding,
                });
            }
        }
        Ok(WindowSelection {
            path: self
                .default_config
                .as_deref()
                .map(|path| select_path(path, agent))
                .transpose()?,
            origin: SelectionOrigin::Default,
        })
    }

    /// Missing files are reported when selected. Persisted metadata is still
    /// readable while a removable drive or an agent project is unavailable.
    pub fn validate(&self) -> Result<()> {
        for path in self
            .default_config
            .iter()
            .chain(self.bindings.keys())
            .chain(self.bindings.values())
        {
            if !path.is_absolute() {
                bail!(
                    "visual config registry paths must be absolute: {}",
                    path.display()
                );
            }
        }
        Ok(())
    }
}

pub(super) fn canonical_agent(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("agent configuration is unavailable: {}", path.display()))
}
