//! Visual documents contain no agent configuration or device/session metadata.

mod registry;
mod selection;
mod storage;

pub use registry::Registry;
pub use selection::{SelectionOrigin, WindowSelection};
pub use storage::{load, save, select_path};

use std::collections::BTreeMap;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::preferences::{
    ExtensionPreferences, FontPreferences, Hints, LabelTruncation, MotionPreference, Preferences,
    SidebarPreferences, ThemeNames, ThemePreference, ViewPreferences,
};

/// An independently selectable appearance and extension arrangement. Omitted
/// fields use built-in values, never model/session settings from the agent.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct VisualConfig {
    pub theme: ThemePreference,
    pub themes: ThemeNames,
    pub fonts: FontPreferences,
    pub motion: MotionPreference,
    pub hints: Hints,
    pub label_truncation: LabelTruncation,
    pub view: ViewPreferences,
    pub extensions: ExtensionPreferences,
    pub sidebar: SidebarPreferences,
    /// Package-owned presentation parameters, separate from agent settings.
    pub package_visual_settings: BTreeMap<String, Map<String, Value>>,
}

impl Default for VisualConfig {
    fn default() -> Self {
        Self::from_preferences(&Preferences::default())
    }
}

impl VisualConfig {
    pub fn from_preferences(value: &Preferences) -> Self {
        Self {
            theme: value.theme,
            themes: value.themes.clone(),
            fonts: value.fonts.clone(),
            motion: value.motion,
            hints: value.hints,
            label_truncation: value.label_truncation,
            view: value.view,
            extensions: value.extensions.clone(),
            sidebar: value.sidebar.clone(),
            package_visual_settings: BTreeMap::new(),
        }
    }

    /// Only the existing built-in visual defaults are updated. In particular,
    /// registry bindings, drivers, sessions, catalogs and input keys survive.
    pub fn apply_to_preferences(&self, value: &mut Preferences) {
        value.theme = self.theme;
        value.themes = self.themes.clone();
        value.fonts = self.fonts.clone();
        value.motion = self.motion;
        value.hints = self.hints;
        value.label_truncation = self.label_truncation;
        value.view = self.view;
        value.extensions = self.extensions.clone();
        value.sidebar = self.sidebar.clone();
    }

    pub fn validate(&self) -> Result<()> {
        self.fonts.validate()?;
        self.view.validate()?;
        self.sidebar.validate()?;
        for (kind, name) in [("dark", &self.themes.dark), ("light", &self.themes.light)] {
            if name.is_empty() || name.trim() != name || name.chars().any(char::is_control) {
                bail!("invalid {kind} theme name");
            }
        }
        for (package, values) in &self.package_visual_settings {
            if package.is_empty()
                || !package
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
            {
                bail!("invalid visual settings package {package:?}");
            }
            validate_package_preferences(values)
                .with_context(|| format!("invalid visual settings for {package}"))?;
        }
        Ok(())
    }
}

/// Check the exact encoded size without allocating another JSON payload.
pub(crate) fn validate_package_preferences(values: &Map<String, Value>) -> Result<()> {
    serde_json::to_writer(PreferenceBytes(0), values)?;
    Ok(())
}

struct PreferenceBytes(usize);

impl std::io::Write for PreferenceBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > 64 * 1024 - self.0 {
            return Err(std::io::Error::other(
                "visual preference map exceeds 64 KiB",
            ));
        }
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
