//! Client preferences stored on this device, independent of agent profiles.

use std::collections::BTreeMap;
use std::fs;
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::recent::RecentWorkspaces;
use crate::sessions::SessionMarks;
use crate::shortcuts::Shortcuts;
use crate::tabs::TabLayout;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    System,
    #[default]
    Dark,
    Light,
}

pub const DEFAULT_DARK_THEME: &str = "Proteus Graphite";
pub const DEFAULT_LIGHT_THEME: &str = "Proteus Light";

/// Colour theme for each appearance, by theme name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeNames {
    pub dark: String,
    pub light: String,
}

impl Default for ThemeNames {
    fn default() -> Self {
        Self {
            dark: DEFAULT_DARK_THEME.to_owned(),
            light: DEFAULT_LIGHT_THEME.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreference {
    #[default]
    Full,
    Reduced,
    Off,
}

/// When a caption's full title appears in its hover tooltip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TitleHint {
    #[default]
    Clipped,
    Always,
    Never,
}

impl TitleHint {
    pub const fn shows(self, clipped: bool) -> bool {
        match self {
            Self::Clipped => clipped,
            Self::Always => true,
            Self::Never => false,
        }
    }
}

/// Independent parts of a hover tooltip, applied immediately on this device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hints {
    pub title: TitleHint,
    pub description: bool,
    pub shortcut: bool,
}

impl Default for Hints {
    fn default() -> Self {
        Self {
            title: TitleHint::Clipped,
            description: true,
            shortcut: true,
        }
    }
}

/// How single-line captions indicate that their text does not fit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelTruncation {
    #[default]
    Fade,
    Ellipsis,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendMode {
    #[default]
    Enter,
    CtrlEnter,
}

/// System notifications about chats while the window is in the background.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notifications {
    #[default]
    Background,
    Off,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub preferred_agent: crate::conversations::AgentKind,
    pub codex_selection: crate::conversations::CodexSelections,
    pub theme: ThemePreference,
    pub themes: ThemeNames,
    pub fonts: FontPreferences,
    pub motion: MotionPreference,
    pub hints: Hints,
    pub label_truncation: LabelTruncation,
    pub send: SendMode,
    pub notifications: Notifications,
    pub view: ViewPreferences,
    pub extensions: ExtensionPreferences,
    pub catalogs: crate::catalog::Preferences,
    pub sidebar: SidebarPreferences,
    pub tabs: TabLayout,
    pub tabs_workspace: Option<TabWorkspace>,
    pub sessions: SessionMarks,
    pub recent: RecentWorkspaces,
    pub shortcuts: Shortcuts,
}

/// The server namespace a saved layout belongs to. Attached connections are
/// deliberately transient: their credentials and origins are never persisted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabWorkspace {
    pub workspace: PathBuf,
    pub profile: Option<String>,
}

impl TabWorkspace {
    pub fn new(workspace: &Path, target: &crate::launch::Target) -> Option<Self> {
        let crate::launch::Target::Spawn { profile } = target else {
            return None;
        };
        Some(Self {
            workspace: workspace.canonicalize().ok()?,
            profile: profile.clone(),
        })
    }
}

impl Preferences {
    /// Unowned or differently scoped tabs cannot be rehosted into this folder.
    pub fn tabs_for(&self, workspace: &Path, target: &crate::launch::Target) -> TabLayout {
        let owner = TabWorkspace::new(workspace, target);
        if owner.is_some() && owner == self.tabs_workspace {
            self.tabs.clone()
        } else {
            TabLayout::default()
        }
    }
}

/// One device-local setting. Reset restores its default or inherited value;
/// unrelated preferences and workspace state are never touched.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preference {
    Theme,
    DarkPalette,
    LightPalette,
    InterfaceFont,
    Scale,
    SurfaceRadius,
    Motion,
    HintTitle,
    HintDescription,
    HintShortcut,
    LabelTruncation,
    Send,
    ChatFont,
    ChatText,
    ChatWidth,
    CompactTools,
    Autoscroll,
    Notifications,
}

impl Preference {
    pub fn is_default(self, prefs: &Preferences) -> bool {
        match self {
            Self::Theme => prefs.theme == ThemePreference::default(),
            Self::DarkPalette => prefs.themes.dark == DEFAULT_DARK_THEME,
            Self::LightPalette => prefs.themes.light == DEFAULT_LIGHT_THEME,
            Self::InterfaceFont => prefs.fonts.interface.is_none(),
            Self::Scale => prefs.view.scale == ViewPreferences::default().scale,
            Self::SurfaceRadius => {
                prefs.view.surface_radius == ViewPreferences::default().surface_radius
            }
            Self::Motion => prefs.motion == MotionPreference::default(),
            Self::HintTitle => prefs.hints.title == Hints::default().title,
            Self::HintDescription => prefs.hints.description == Hints::default().description,
            Self::HintShortcut => prefs.hints.shortcut == Hints::default().shortcut,
            Self::LabelTruncation => prefs.label_truncation == LabelTruncation::default(),
            Self::Send => prefs.send == SendMode::default(),
            Self::ChatFont => prefs.fonts.chat.is_none(),
            Self::ChatText => prefs.view.chat_text == ViewPreferences::default().chat_text,
            Self::ChatWidth => prefs.view.chat_width == ViewPreferences::default().chat_width,
            Self::CompactTools => {
                prefs.view.compact_tools == ViewPreferences::default().compact_tools
            }
            Self::Autoscroll => prefs.view.autoscroll == ViewPreferences::default().autoscroll,
            Self::Notifications => prefs.notifications == Notifications::default(),
        }
    }

    pub fn reset(self, prefs: &mut Preferences) {
        match self {
            Self::Theme => prefs.theme = ThemePreference::default(),
            Self::DarkPalette => prefs.themes.dark = DEFAULT_DARK_THEME.into(),
            Self::LightPalette => prefs.themes.light = DEFAULT_LIGHT_THEME.into(),
            Self::InterfaceFont => prefs.fonts.interface = None,
            Self::Scale => prefs.view.scale = ViewPreferences::default().scale,
            Self::SurfaceRadius => {
                prefs.view.surface_radius = ViewPreferences::default().surface_radius
            }
            Self::Motion => prefs.motion = MotionPreference::default(),
            Self::HintTitle => prefs.hints.title = Hints::default().title,
            Self::HintDescription => prefs.hints.description = Hints::default().description,
            Self::HintShortcut => prefs.hints.shortcut = Hints::default().shortcut,
            Self::LabelTruncation => prefs.label_truncation = LabelTruncation::default(),
            Self::Send => prefs.send = SendMode::default(),
            Self::ChatFont => prefs.fonts.chat = None,
            Self::ChatText => prefs.view.chat_text = ViewPreferences::default().chat_text,
            Self::ChatWidth => prefs.view.chat_width = ViewPreferences::default().chat_width,
            Self::CompactTools => {
                prefs.view.compact_tools = ViewPreferences::default().compact_tools
            }
            Self::Autoscroll => prefs.view.autoscroll = ViewPreferences::default().autoscroll,
            Self::Notifications => prefs.notifications = Notifications::default(),
        }
    }
}

/// Optional installed families; `None` inherits the theme/interface family.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FontPreferences {
    pub interface: Option<String>,
    pub chat: Option<String>,
}

impl FontPreferences {
    fn validate(&self) -> Result<()> {
        for (field, family) in [("interface", &self.interface), ("chat", &self.chat)] {
            if let Some(family) = family
                && (family.is_empty()
                    || family.len() > 256
                    || family.trim() != family
                    || family.chars().any(char::is_control))
            {
                return Err(anyhow!("invalid {field} font family"));
            }
        }
        Ok(())
    }
}

/// Interface scale steps in percent, for the menu and Ctrl+=/Ctrl+-.
pub const SCALES: [u16; 9] = [75, 80, 90, 100, 110, 125, 150, 175, 200];
/// Text size of chat messages and the composer, px at 100%.
pub const CHAT_TEXT: RangeInclusive<u16> = 12..=22;
/// Widest conversation column, px at 100%.
pub const CHAT_WIDTH: RangeInclusive<u16> = 420..=1600;
/// Radius of large surfaces, px at 100%.
pub const SURFACE_RADIUS: RangeInclusive<u16> = 0..=32;

/// Sizes of the interface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ViewPreferences {
    /// Interface scale in percent, one of [`SCALES`].
    pub scale: u16,
    pub chat_text: u16,
    pub chat_width: u16,
    /// Radius of large surfaces, px at 100%.
    pub surface_radius: u16,
    /// Fold runs of tool calls into one row.
    pub compact_tools: bool,
    /// Follow a streaming answer to the end of the chat.
    pub autoscroll: bool,
}

impl Default for ViewPreferences {
    fn default() -> Self {
        Self {
            scale: 100,
            chat_text: 16,
            chat_width: 820,
            surface_radius: 16,
            compact_tools: true,
            autoscroll: true,
        }
    }
}

impl ViewPreferences {
    pub fn scale_factor(&self) -> f32 {
        f32::from(self.scale) / 100.
    }

    /// Moves the scale `steps` positions along [`SCALES`]; 0 resets it.
    pub fn zoom(&mut self, steps: i32) {
        if steps == 0 {
            self.scale = Self::default().scale;
            return;
        }
        let at = SCALES
            .iter()
            .position(|scale| *scale >= self.scale)
            .unwrap_or(SCALES.len() - 1) as i32;
        let to = (at + steps).clamp(0, SCALES.len() as i32 - 1);
        self.scale = SCALES[to as usize];
    }

    fn validate(&self) -> Result<()> {
        if !SCALES.contains(&self.scale) {
            return Err(anyhow!("scale {} is not one of {SCALES:?}", self.scale));
        }
        if !CHAT_TEXT.contains(&self.chat_text) {
            return Err(anyhow!(
                "chat_text {} is outside {CHAT_TEXT:?}",
                self.chat_text
            ));
        }
        if !CHAT_WIDTH.contains(&self.chat_width) {
            return Err(anyhow!(
                "chat_width {} is outside {CHAT_WIDTH:?}",
                self.chat_width
            ));
        }
        if !SURFACE_RADIUS.contains(&self.surface_radius) {
            return Err(anyhow!(
                "surface_radius {} is outside {SURFACE_RADIUS:?}",
                self.surface_radius
            ));
        }
        Ok(())
    }
}

/// Panel of the session list; other panels are named by their extension.
pub const SESSIONS_PANEL: &str = "sessions";
/// Width of the side panel, px at 100%.
pub const SIDEBAR_WIDTH: RangeInclusive<f32> = 180.0..=640.0;

/// The side panel between the rail and the tabs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SidebarPreferences {
    /// Panel shown: [`SESSIONS_PANEL`] or an extension id; none while the
    /// side panel is closed.
    pub panel: Option<String>,
    pub width: f32,
}

impl Default for SidebarPreferences {
    fn default() -> Self {
        Self {
            panel: Some(SESSIONS_PANEL.to_owned()),
            width: 264.,
        }
    }
}

impl SidebarPreferences {
    fn validate(&self) -> Result<()> {
        if !SIDEBAR_WIDTH.contains(&self.width) {
            return Err(anyhow!("width {} is outside {SIDEBAR_WIDTH:?}", self.width));
        }
        Ok(())
    }
}

/// Where the compact icon of an extension is shown: the left or the right
/// end of the composer row and the top bar, or the top or the bottom of the
/// rail along the left edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BadgePlace {
    ComposerStart,
    ComposerEnd,
    HeaderStart,
    HeaderEnd,
    RailStart,
    RailEnd,
    Hidden,
}

impl BadgePlace {
    /// Every place where icons are drawn.
    pub const SHOWN: [BadgePlace; 6] = [
        BadgePlace::ComposerStart,
        BadgePlace::ComposerEnd,
        BadgePlace::HeaderStart,
        BadgePlace::HeaderEnd,
        BadgePlace::RailStart,
        BadgePlace::RailEnd,
    ];

    /// Icons here stand in a column, so only narrow ones fit.
    pub fn is_rail(self) -> bool {
        matches!(self, BadgePlace::RailStart | BadgePlace::RailEnd)
    }
}

/// Which client extensions are on, and where their icons are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExtensionPreferences {
    /// Extension order in the settings list and their pages.
    pub order: Vec<String>,
    pub disabled: Vec<String>,
    /// Icon places chosen by the user; others use the extension default.
    pub badges: BTreeMap<String, BadgePlace>,
    pub badge_order: Vec<String>,
    /// Icons fixed in place: they cannot be dragged.
    pub pinned: Vec<String>,
}

impl Default for ExtensionPreferences {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            disabled: vec!["analysis".into()],
            badges: BTreeMap::new(),
            badge_order: Vec::new(),
            pinned: Vec::new(),
        }
    }
}

impl ExtensionPreferences {
    pub fn is_enabled(&self, id: &str) -> bool {
        !self.disabled.iter().any(|disabled| disabled == id)
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        self.disabled.retain(|disabled| disabled != id);
        if !enabled {
            self.disabled.push(id.to_string());
        }
    }

    /// Extensions in the saved order; ids it does not mention yet follow in
    /// catalog order.
    pub fn ordered<'a>(&self, known: &[&'a str]) -> Vec<&'a str> {
        ordered(&self.order, known)
    }

    /// Moves an extension in front of `before` or to the end of the list.
    pub fn move_extension(&mut self, known: &[&str], id: &str, before: Option<&str>) {
        move_before(&mut self.order, known, id, before);
    }

    /// Drops everything remembered about a removed extension.
    pub fn forget(&mut self, id: &str) {
        for list in [
            &mut self.order,
            &mut self.disabled,
            &mut self.badge_order,
            &mut self.pinned,
        ] {
            list.retain(|item| item != id);
        }
        self.badges.remove(id);
    }

    pub fn badge_place(&self, id: &str, default: BadgePlace) -> BadgePlace {
        self.badges.get(id).copied().unwrap_or(default)
    }

    pub fn set_badge_place(&mut self, id: &str, place: BadgePlace) {
        self.badges.insert(id.to_string(), place);
    }

    /// Icons in the saved order; ids the order does not mention yet follow
    /// in catalog order.
    pub fn badges_ordered<'a>(&self, known: &[&'a str]) -> Vec<&'a str> {
        ordered(&self.badge_order, known)
    }

    /// Puts an icon into `place`, in front of `before` or at its end.
    pub fn move_badge(
        &mut self,
        known: &[&str],
        id: &str,
        place: BadgePlace,
        before: Option<&str>,
    ) {
        self.set_badge_place(id, place);
        move_before(&mut self.badge_order, known, id, before);
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.iter().any(|pinned| pinned == id)
    }

    pub fn set_pinned(&mut self, id: &str, pinned: bool) {
        self.pinned.retain(|item| item != id);
        if pinned {
            self.pinned.push(id.to_string());
        }
    }
}

fn ordered<'a>(saved: &[String], known: &[&'a str]) -> Vec<&'a str> {
    let mut ordered: Vec<&'a str> = saved
        .iter()
        .filter_map(|id| known.iter().find(|known| *known == id).copied())
        .collect();
    for id in known {
        if !ordered.contains(id) {
            ordered.push(id);
        }
    }
    ordered
}

fn move_before(saved: &mut Vec<String>, known: &[&str], id: &str, before: Option<&str>) {
    if before == Some(id) {
        return;
    }
    let mut items = ordered(saved, known);
    let Some(from) = items.iter().position(|item| *item == id) else {
        return;
    };
    let item = items.remove(from);
    let to = before
        .and_then(|before| items.iter().position(|item| *item == before))
        .unwrap_or(items.len());
    items.insert(to, item);
    *saved = items.into_iter().map(str::to_string).collect();
}

/// Directory of the client's files on this device.
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join(crate::APP_ID))
}

fn path() -> Option<PathBuf> {
    Some(config_dir()?.join("preferences.json"))
}

/// Saved preferences; a missing file means defaults. A file that cannot be
/// used is moved aside, so the next save does not overwrite it.
pub fn load() -> Result<Preferences> {
    let Some(path) = path() else {
        return Ok(Preferences::default());
    };
    match fs::read_to_string(&path) {
        Ok(text) => parse(&text, &path).map_err(|error| match set_aside(&path) {
            Ok(kept) => error.context(format!("the file was moved to {}", kept.display())),
            Err(_) => error,
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn parse(text: &str, path: &Path) -> Result<Preferences> {
    let preferences =
        decode_saved(text).with_context(|| format!("invalid preferences in {}", path.display()))?;
    preferences
        .tabs
        .validate()
        .map_err(|error| anyhow!("invalid tab layout in {}: {error}", path.display()))?;
    preferences
        .view
        .validate()
        .with_context(|| format!("invalid view sizes in {}", path.display()))?;
    preferences
        .fonts
        .validate()
        .with_context(|| format!("invalid fonts in {}", path.display()))?;
    preferences
        .sidebar
        .validate()
        .with_context(|| format!("invalid side panel in {}", path.display()))?;
    preferences
        .shortcuts
        .validate()
        .with_context(|| format!("invalid shortcuts in {}", path.display()))?;
    Ok(preferences)
}

/// Upgrade the former tooltip mode only at the saved-file boundary. The public
/// typed format accepts the current object exclusively and writes no old aliases.
fn decode_saved(text: &str) -> serde_json::Result<Preferences> {
    let original = match serde_json::from_str(text) {
        Ok(preferences) => return Ok(preferences),
        Err(error) => error,
    };
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    let mode = value.get("hints").and_then(serde_json::Value::as_str);
    let (title, description, shortcut) = match mode {
        Some("full") => (TitleHint::Clipped, true, true),
        Some("description") => (TitleHint::Clipped, true, false),
        Some("keys") => (TitleHint::Never, false, true),
        Some("off") => (TitleHint::Never, false, false),
        _ => return Err(original),
    };
    value["hints"] = serde_json::to_value(Hints {
        title,
        description,
        shortcut,
    })?;
    serde_json::from_value(value)
}

fn set_aside(path: &Path) -> std::io::Result<PathBuf> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let kept = path.with_extension(format!("json.broken-{stamp}"));
    fs::rename(path, &kept)?;
    Ok(kept)
}

/// Writes a sibling file first, so a crash never leaves half a file.
pub fn save(preferences: &Preferences) -> Result<()> {
    let path = path().context("no HOME or XDG_CONFIG_HOME for preferences")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("failed to create {}", dir.display()))?;
    }
    let text = serde_json::to_string_pretty(preferences)?;
    let draft = path.with_extension("json.tmp");
    fs::write(&draft, text).with_context(|| format!("failed to write {}", draft.display()))?;
    fs::rename(&draft, &path).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{BadgePlace, ExtensionPreferences, Preferences, SURFACE_RADIUS, ViewPreferences};

    const KNOWN: [&str; 3] = ["context", "plan", "files"];

    #[test]
    fn saved_tabs_restore_only_in_their_folder_and_profile() {
        use super::TabWorkspace;
        use crate::launch::Target;
        let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let target = Target::Spawn {
            profile: Some("fixture".into()),
        };
        let mut prefs = Preferences::default();
        prefs.tabs.open("file:relative/source.rs", |_| false);
        // Ownerless preferences are not guessed into the current server.
        assert!(
            !prefs
                .tabs_for(workspace, &target)
                .contains("file:relative/source.rs")
        );
        prefs.tabs_workspace = TabWorkspace::new(workspace, &target);
        assert!(
            prefs
                .tabs_for(workspace, &target)
                .contains("file:relative/source.rs")
        );
        assert!(
            !prefs
                .tabs_for(workspace.parent().unwrap(), &target)
                .contains("file:relative/source.rs")
        );
        assert!(
            !prefs
                .tabs_for(workspace, &Target::Spawn { profile: None })
                .contains("file:relative/source.rs")
        );
        let attached = Target::Attach {
            origin: "http://localhost:9".into(),
            token: "never-persist-this".into(),
        };
        assert!(TabWorkspace::new(workspace, &attached).is_none());
        assert!(
            !prefs
                .tabs_for(workspace, &attached)
                .contains("file:relative/source.rs")
        );
        let encoded = serde_json::to_string(&prefs).unwrap();
        assert!(!encoded.contains("never-persist-this"));
        let reopened: Preferences = serde_json::from_str(&encoded).unwrap();
        assert!(
            reopened
                .tabs_for(workspace, &target)
                .contains("file:relative/source.rs")
        );
        // Canonically equivalent folder names share the layout.
        assert!(
            reopened
                .tabs_for(&workspace.join("."), &target)
                .contains("file:relative/source.rs")
        );
    }

    #[test]
    fn analysis_starts_disabled_but_explicit_enable_survives_reopen() {
        let mut prefs: Preferences = serde_json::from_str("{}").unwrap();
        assert!(!prefs.extensions.is_enabled("analysis"));
        assert!(prefs.extensions.is_enabled("plan"));
        prefs.extensions.set_enabled("analysis", true);
        let reopened: Preferences =
            serde_json::from_str(&serde_json::to_string(&prefs).unwrap()).unwrap();
        assert!(reopened.extensions.is_enabled("analysis"));
        let explicit: Preferences =
            serde_json::from_str(r#"{"extensions":{"disabled":[]}}"#).unwrap();
        assert!(explicit.extensions.is_enabled("analysis"));
    }

    #[test]
    fn truncation_defaults_for_old_files_and_roundtrips() {
        let legacy = super::parse("{}", std::path::Path::new("fixture.json")).unwrap();
        assert_eq!(legacy.label_truncation, super::LabelTruncation::Fade);
        let mut value = legacy;
        value.label_truncation = super::LabelTruncation::Ellipsis;
        let reopened = super::parse(
            &serde_json::to_string(&value).unwrap(),
            std::path::Path::new("fixture.json"),
        )
        .unwrap();
        assert_eq!(reopened.label_truncation, super::LabelTruncation::Ellipsis);
    }

    #[test]
    fn title_hint_modes_respect_actual_clipping() {
        use super::TitleHint::*;
        for (mode, clipped, shown) in [
            (Clipped, false, false),
            (Clipped, true, true),
            (Always, false, true),
            (Always, true, true),
            (Never, false, false),
            (Never, true, false),
        ] {
            assert_eq!(mode.shows(clipped), shown, "{mode:?}, clipped={clipped}");
        }
    }

    #[test]
    fn tooltip_parts_default_and_roundtrip_independently() {
        use super::{Hints, TitleHint};
        let path = std::path::Path::new("fixture.json");
        let default = super::parse("{}", path).unwrap().hints;
        assert_eq!(default.title, TitleHint::Clipped);
        assert!(default.description);
        assert!(default.shortcut);
        assert_eq!(
            super::parse(r#"{"hints":{}}"#, path).unwrap().hints,
            default
        );
        for title in [TitleHint::Clipped, TitleHint::Always, TitleHint::Never] {
            for description in [false, true] {
                for shortcut in [false, true] {
                    let expected = Hints {
                        title,
                        description,
                        shortcut,
                    };
                    let mut prefs = Preferences::default();
                    prefs.hints = expected;
                    let text = serde_json::to_string(&prefs).unwrap();
                    assert_eq!(super::parse(&text, path).unwrap().hints, expected);
                }
            }
        }
    }

    #[test]
    fn saved_tooltip_modes_migrate_without_losing_other_preferences() {
        let path = std::path::Path::new("fixture.json");
        for (mode, title, description, shortcut) in [
            ("full", super::TitleHint::Clipped, true, true),
            ("description", super::TitleHint::Clipped, true, false),
            ("keys", super::TitleHint::Never, false, true),
            ("off", super::TitleHint::Never, false, false),
        ] {
            let source = serde_json::json!({
                "hints": mode,
                "theme": "light",
                "fonts": {"chat": "Example font"},
                "sidebar": {"width": 333.0},
                "extensions": {"disabled": ["analysis", "plan"]},
            });
            let prefs = super::parse(&source.to_string(), path).unwrap();
            let mut expected = Preferences::default();
            expected.hints.title = title;
            expected.hints.description = description;
            expected.hints.shortcut = shortcut;
            expected.theme = super::ThemePreference::Light;
            expected.fonts.chat = Some("Example font".into());
            expected.sidebar.width = 333.;
            expected.extensions.disabled = vec!["analysis".into(), "plan".into()];
            assert_eq!(prefs, expected, "{mode}");
            let canonical = serde_json::to_value(&prefs).unwrap();
            assert!(canonical["hints"].is_object());
            assert_eq!(
                super::parse(&canonical.to_string(), path).unwrap(),
                expected
            );
            assert!(serde_json::from_value::<super::Hints>(source["hints"].clone()).is_err());
        }
    }

    #[test]
    fn malformed_tooltips_and_unrelated_corruption_still_fail() {
        let path = std::path::Path::new("fixture.json");
        for source in [
            r#"{"hints":"unknown"}"#,
            r#"{"hints":null}"#,
            r#"{"hints":{"title":"sometimes"}}"#,
            r#"{"hints":{"description":"false"}}"#,
            r#"{"hints":{"shortcut":0}}"#,
            r#"{"hints":{"unknown":true}}"#,
            r#"{"hints":"off","view":{"scale":13}}"#,
            r#"{"hints":"full","unknown":true}"#,
        ] {
            assert!(super::parse(source, path).is_err(), "{source}");
        }
    }

    #[test]
    fn each_setting_reset_changes_only_its_value_and_preserves_workspace_state() {
        use super::Preference::*;
        use serde_json::{Value, json};

        let fields = [
            (Theme, "/theme", json!("light")),
            (DarkPalette, "/themes/dark", json!("Custom dark")),
            (LightPalette, "/themes/light", json!("Custom light")),
            (InterfaceFont, "/fonts/interface", json!("Interface")),
            (Scale, "/view/scale", json!(150)),
            (SurfaceRadius, "/view/surface_radius", json!(4)),
            (Motion, "/motion", json!("off")),
            (HintTitle, "/hints/title", json!("always")),
            (HintDescription, "/hints/description", json!(false)),
            (HintShortcut, "/hints/shortcut", json!(false)),
            (LabelTruncation, "/label_truncation", json!("ellipsis")),
            (Send, "/send", json!("ctrl_enter")),
            (ChatFont, "/fonts/chat", json!("Chat")),
            (ChatText, "/view/chat_text", json!(20)),
            (ChatWidth, "/view/chat_width", json!(1200)),
            (CompactTools, "/view/compact_tools", json!(false)),
            (Autoscroll, "/view/autoscroll", json!(false)),
            (Notifications, "/notifications", json!("off")),
        ];
        let defaults = Preferences::default();
        let default_json = serde_json::to_value(&defaults).unwrap();
        let mut modified = default_json.clone();
        for (field, path, changed) in &fields {
            assert!(field.is_default(&defaults), "{field:?}");
            *modified.pointer_mut(path).unwrap() = changed.clone();
        }
        *modified.pointer_mut("/sidebar/width").unwrap() = json!(333.0);
        *modified.pointer_mut("/extensions/disabled").unwrap() = json!(["plan", "analysis"]);
        let modified_prefs: Preferences = serde_json::from_value(modified.clone()).unwrap();
        for (field, path, _) in &fields {
            let mut prefs = modified_prefs.clone();
            assert!(!field.is_default(&prefs), "{field:?}");
            field.reset(&mut prefs);
            assert!(field.is_default(&prefs), "{field:?}");
            let mut expected: Value = modified.clone();
            *expected.pointer_mut(path).unwrap() = default_json.pointer(path).unwrap().clone();
            assert_eq!(serde_json::to_value(&prefs).unwrap(), expected, "{field:?}");
            field.reset(&mut prefs);
            assert_eq!(
                serde_json::to_value(&prefs).unwrap(),
                expected,
                "idempotent {field:?}"
            );
            let restored: Preferences = serde_json::from_value(expected).unwrap();
            assert_eq!(restored, prefs);
        }
    }

    #[test]
    fn old_preferences_inherit_fonts_and_independent_choices_roundtrip() {
        let legacy = super::parse(
            r#"{"view":{"chat_text":18}}"#,
            std::path::Path::new("fixture.json"),
        )
        .unwrap();
        assert_eq!(legacy.fonts, super::FontPreferences::default());
        assert_eq!(legacy.view.chat_text, 18);

        let mut value = Preferences::default();
        value.fonts.interface = Some("Noto Sans".into());
        value.fonts.chat = Some("Шрифт беседы".into());
        let serialized = serde_json::to_string(&value).unwrap();
        let reopened = super::parse(&serialized, std::path::Path::new("fixture.json")).unwrap();
        assert_eq!(reopened.fonts, value.fonts);
        value.fonts.interface = None;
        let reopened = super::parse(
            &serde_json::to_string(&value).unwrap(),
            std::path::Path::new("fixture.json"),
        )
        .unwrap();
        assert_eq!(reopened.fonts.interface, None);
        assert_eq!(reopened.fonts.chat.as_deref(), Some("Шрифт беседы"));
        value.fonts.chat = None;
        let reopened = super::parse(
            &serde_json::to_string(&value).unwrap(),
            std::path::Path::new("fixture.json"),
        )
        .unwrap();
        assert_eq!(reopened.fonts, super::FontPreferences::default());
    }

    #[test]
    fn malformed_font_names_are_rejected_but_unavailable_family_remains_portable() {
        for name in [
            "",
            " Noto Sans",
            "Noto Sans ",
            "Noto\nSans",
            "Noto\0Sans",
            &"x".repeat(257),
        ] {
            for key in ["interface", "chat"] {
                let source = serde_json::json!({"fonts":{key:name}}).to_string();
                assert!(
                    super::parse(&source, std::path::Path::new("fixture.json")).is_err(),
                    "{source}"
                );
            }
        }
        // Device-specific availability belongs to native font resolution,
        // not file validity: moving a profile must not quarantine it.
        let saved = super::parse(
            r#"{"fonts":{"chat":"Uninstalled synthetic family"}}"#,
            std::path::Path::new("fixture.json"),
        )
        .unwrap();
        assert_eq!(
            saved.fonts.chat.as_deref(),
            Some("Uninstalled synthetic family")
        );
    }

    #[test]
    fn switches_and_pins_toggle() {
        let mut extensions = ExtensionPreferences::default();
        extensions.set_enabled("files", false);
        assert!(!extensions.is_enabled("files"));
        extensions.set_enabled("files", true);
        assert!(extensions.is_enabled("files"));
        assert_eq!(extensions.disabled, ["analysis"]);
        extensions.set_pinned("plan", true);
        extensions.set_pinned("plan", true);
        assert!(extensions.is_pinned("plan") && extensions.pinned.len() == 1);
        extensions.set_pinned("plan", false);
        assert!(!extensions.is_pinned("plan"));
    }

    #[test]
    fn badges_move_between_places() {
        let mut extensions = ExtensionPreferences::default();
        assert_eq!(extensions.badges_ordered(&KNOWN), KNOWN);
        assert_eq!(
            extensions.badge_place("plan", BadgePlace::ComposerStart),
            BadgePlace::ComposerStart
        );
        extensions.move_badge(&KNOWN, "files", BadgePlace::HeaderEnd, Some("context"));
        assert_eq!(
            extensions.badge_place("files", BadgePlace::ComposerStart),
            BadgePlace::HeaderEnd
        );
        assert_eq!(
            extensions.badges_ordered(&KNOWN),
            ["files", "context", "plan"]
        );
        // Dropping an icon onto itself keeps the order.
        extensions.move_badge(&KNOWN, "plan", BadgePlace::ComposerStart, Some("plan"));
        assert_eq!(
            extensions.badges_ordered(&KNOWN),
            ["files", "context", "plan"]
        );
        // An extension added later joins at the end; a removed one is skipped.
        assert_eq!(
            extensions.badges_ordered(&["plan", "quota", "files"]),
            ["files", "plan", "quota"]
        );
    }

    #[test]
    fn extensions_reorder_and_are_forgotten_on_removal() {
        let mut extensions = ExtensionPreferences::default();
        extensions.move_extension(&KNOWN, "files", Some("context"));
        assert_eq!(extensions.ordered(&KNOWN), ["files", "context", "plan"]);
        extensions.set_enabled("files", false);
        extensions.move_badge(&KNOWN, "files", BadgePlace::HeaderEnd, None);
        extensions.set_pinned("files", true);
        extensions.forget("files");
        assert_eq!(extensions.ordered(&KNOWN), ["context", "plan", "files"]);
        assert!(extensions.is_enabled("files") && !extensions.is_pinned("files"));
        assert!(extensions.badges.is_empty() && extensions.badge_order.len() == 2);
    }

    #[test]
    fn zoom_walks_the_scale_steps() {
        let mut view = ViewPreferences::default();
        view.zoom(1);
        assert_eq!(view.scale, 110);
        view.zoom(-3);
        assert_eq!(view.scale, 80);
        view.zoom(-5);
        assert_eq!(view.scale, 75);
        view.zoom(20);
        assert_eq!(view.scale, 200);
        view.zoom(0);
        assert_eq!(view.scale, 100);
        assert!(view.validate().is_ok());
        view.chat_width = 100;
        assert!(view.validate().is_err());
    }

    #[test]
    fn surface_radius_defaults_validates_and_roundtrips() {
        let defaults = ViewPreferences::default();
        assert_eq!(defaults.surface_radius, 16);
        assert!(defaults.validate().is_ok());

        // Older preference files omit the new field and keep its default.
        let legacy: Preferences = serde_json::from_str(
            r#"{"view":{"scale":100,"chat_text":16,"chat_width":820,"compact_tools":true,"autoscroll":true}}"#,
        )
        .unwrap();
        assert_eq!(legacy.view.surface_radius, 16);

        for radius in [*SURFACE_RADIUS.start(), 16, *SURFACE_RADIUS.end()] {
            let mut preferences = Preferences::default();
            preferences.view.surface_radius = radius;
            assert!(preferences.view.validate().is_ok());
            let serialized = serde_json::to_string(&preferences).unwrap();
            let roundtrip: Preferences = serde_json::from_str(&serialized).unwrap();
            assert_eq!(roundtrip, preferences);
        }

        let mut invalid = defaults;
        invalid.surface_radius = *SURFACE_RADIUS.end() + 1;
        assert!(invalid.validate().is_err());
    }
}
