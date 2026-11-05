//! `extension.json` of an installed extension and the folder scan.

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::preferences::BadgePlace;

pub const MANIFEST: &str = "extension.json";
const TAGS: &str = "org.proteus/tags";

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub protocol: u32,
    #[serde(default)]
    pub required_features: Vec<String>,
    /// Equals the folder name; tab ids and preferences use it.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// Icon name from the client's set; unknown names get the default.
    #[serde(default)]
    pub icon: Option<String>,
    /// Program and arguments, started in the extension folder.
    pub command: Vec<String>,
    /// Subscribe to the active session's state and optional transcript projection.
    #[serde(default)]
    pub session_events: bool,
    /// State-only widgets can opt out before tool payloads are copied/encoded.
    #[serde(default = "session_entries_default")]
    pub session_entries: bool,
    #[serde(default)]
    pub dependencies: Dependencies,
    /// Packaged skill directories linked from settings. They do not install
    /// skills, change the agent prompt, or create runtime dependencies.
    #[serde(default)]
    pub agent_instructions: Vec<AgentInstruction>,
    pub surfaces: Surfaces,
    /// Optional author-namespaced descriptive data; it never enables behavior.
    #[serde(default, rename = "_meta")]
    pub metadata: proteus_contracts::extension_metadata::ExtensionMetadata,
}

impl Manifest {
    pub fn accepts_session_update(&self, update: &super::session::Update) -> bool {
        self.session_events
            && (self.session_entries || matches!(update, super::session::Update::State { .. }))
    }
    /// Descriptive labels only: these never grant API access or capabilities.
    pub fn tags(&self) -> impl Iterator<Item = &str> {
        self.metadata
            .0
            .get(TAGS)
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
    }
}

fn session_entries_default() -> bool {
    true
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Surfaces {
    /// Inline expandable contribution above the composer. It uses the
    /// existing badge presentation and popup content wire surfaces.
    #[serde(default)]
    pub activity: Option<ActivitySurface>,
    /// Explicit, synthetic, view-owned preview in extension settings.
    #[serde(default)]
    pub preview: Option<PreviewSurface>,
    /// Fenced assistant-message languages this package can render.
    #[serde(default)]
    pub content: Option<ContentSurface>,
    #[serde(default)]
    pub page: Option<PageSurface>,
    #[serde(default)]
    pub popup: Option<PopupSurface>,
    #[serde(default)]
    pub panel: Option<PanelSurface>,
    #[serde(default)]
    pub document: Option<DocumentSurface>,
    #[serde(default)]
    pub badge: Option<BadgeSurface>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ContentSurface {
    #[serde(default)]
    pub languages: Vec<String>,
    /// Full-body media types. The first enabled contributor wins.
    #[serde(default)]
    pub formats: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PopupSurface {
    #[serde(default)]
    pub escape_action: Option<String>,
    #[serde(default = "popup_width")]
    pub width: f32,
    /// The tallest the popup grows before its content scrolls.
    #[serde(default = "popup_height")]
    pub height: f32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PageSurface {
    /// Settings controls embedded in the host page's single scroll area.
    #[serde(default)]
    pub embedded: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActivitySurface {}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PreviewSurface {}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Dependencies {
    /// Requires a Proteus Core conversation. Local surfaces opt out and
    /// receive a folder without Core credentials or session projection.
    #[serde(default = "core_dependency_default")]
    pub core: bool,
    /// Tools required to create new content. Missing tools do not prevent
    /// reading saved content; the settings shell links to their controls.
    #[serde(default)]
    pub tools: Vec<String>,
}

fn core_dependency_default() -> bool {
    true
}

impl Default for Dependencies {
    fn default() -> Self {
        Self {
            core: true,
            tools: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentInstruction {
    /// Skill identifier, including a namespace when one is used.
    pub skill: String,
    /// Package-relative directory containing SKILL.md.
    pub path: PathBuf,
}

fn popup_width() -> f32 {
    280.
}

fn popup_height() -> f32 {
    420.
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PanelSurface {
    pub title: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DocumentSurface {
    /// Template supports {argument} and {filename}.
    #[serde(default)]
    pub label: Option<String>,
    /// Contribution address used by client commands, e.g. terminal.new.
    #[serde(default)]
    pub command: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BadgeSurface {
    /// Wait for the extension's first presentation instead of flashing an empty icon.
    #[serde(default)]
    pub initially_hidden: bool,
    #[serde(default)]
    pub narrow: bool,
    #[serde(default = "composer_start")]
    pub place: BadgePlace,
}

fn composer_start() -> BadgePlace {
    BadgePlace::ComposerStart
}

/// An extension found on disk.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub revision: u64,
}

/// A folder that looks like an extension but cannot be loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub dir: PathBuf,
    pub error: String,
}

/// Where the user keeps extensions.
pub fn root() -> Option<PathBuf> {
    Some(crate::preferences::config_dir()?.join("extensions"))
}

/// Portable assets or the source checkout used by `cargo run`.
pub fn bundled_root() -> PathBuf {
    if let Some(path) = std::env::var_os("PROTEUS_EXTENSIONS_DIR") {
        return path.into();
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let native = Path::new(env!("CARGO_MANIFEST_DIR"));
    if exe.starts_with(native.join("target")) {
        let local = native.join("../Proteus-desktop-extensions/packages");
        return if local.is_dir() {
            local
        } else {
            native.join("build/sources/extensions/packages")
        };
    }
    exe.parent()
        .and_then(Path::parent)
        .unwrap_or(Path::new("."))
        .join("share/extensions")
}

/// A user installation overrides a bundled package, including a broken
/// installation: show its error instead of silently running another version.
pub fn discover_sources(bundled: &Path, user: Option<&Path>) -> (Vec<Installed>, Vec<Problem>) {
    let (mut found, mut problems) = discover(bundled);
    if let Some(user) = user {
        found.retain(|extension| !user.join(&extension.manifest.id).exists());
        let (installed, errors) = discover(user);
        found.extend(installed);
        problems.extend(errors);
    }
    (found, problems)
}

/// Reads every `<root>/<id>/extension.json`, sorted by folder name. A
/// missing root means no extensions.
pub fn discover(root: &Path) -> (Vec<Installed>, Vec<Problem>) {
    let mut installed = Vec::new();
    let mut problems = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return (installed, problems);
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir() && !is_hidden(path))
        .collect();
    dirs.sort();
    for dir in dirs {
        match load(&dir) {
            Ok(manifest) => installed.push(Installed {
                revision: revision(&dir),
                dir,
                manifest,
            }),
            Err(error) => problems.push(Problem {
                dir,
                error: format!("{error:#}"),
            }),
        }
    }
    (installed, problems)
}

/// Detect package replacement and source edits without rebuilding the client.
fn revision(dir: &Path) -> u64 {
    fn visit(dir: &Path, hash: &mut impl Hasher) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<_> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        paths.sort();
        for path in paths {
            if is_hidden(&path) || path.file_name().is_some_and(|name| name == "__pycache__") {
                continue;
            }
            path.hash(hash);
            if let Ok(metadata) = path.metadata() {
                metadata.len().hash(hash);
                metadata.modified().ok().hash(hash);
                if metadata.is_dir() && !path.is_symlink() {
                    visit(&path, hash);
                }
            }
        }
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    visit(dir, &mut hash);
    hash.finish()
}

/// Dot folders are unfinished installs and anything else the user hid.
fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

pub(crate) fn load(dir: &Path) -> Result<Manifest> {
    let path = dir.join(MANIFEST);
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let manifest = parse(&text).with_context(|| format!("invalid {}", path.display()))?;
    let folder = dir.file_name().and_then(|name| name.to_str()).unwrap_or("");
    if manifest.id != folder {
        bail!("id `{}` must equal the folder name `{folder}`", manifest.id);
    }
    if !manifest.agent_instructions.is_empty() {
        let package = dir.canonicalize()?;
        let mut seen = std::collections::HashSet::new();
        for instruction in &manifest.agent_instructions {
            let skill_dir = dir
                .join(&instruction.path)
                .canonicalize()
                .with_context(|| {
                    format!(
                        "cannot resolve packaged skill {}",
                        instruction.path.display()
                    )
                })?;
            let skill_file = skill_dir
                .join("SKILL.md")
                .canonicalize()
                .with_context(|| format!("cannot resolve SKILL.md for {}", instruction.skill))?;
            if !skill_dir.starts_with(&package)
                || !skill_dir.is_dir()
                || !skill_file.starts_with(&package)
                || !skill_file.is_file()
                || !seen.insert(skill_dir)
            {
                bail!(
                    "agent instructions must reference distinct skill directories with regular SKILL.md files inside the package"
                );
            }
        }
    }
    Ok(manifest)
}

/// Reads and checks an `extension.json` text.
pub fn parse(text: &str) -> Result<Manifest> {
    let manifest: Manifest = serde_json::from_str(text)?;
    if manifest.protocol != super::protocol::PROTOCOL {
        bail!(
            "unsupported extension protocol {}; expected {}",
            manifest.protocol,
            super::protocol::PROTOCOL
        );
    }
    let mut required = std::collections::BTreeSet::new();
    for feature in &manifest.required_features {
        if !super::protocol::FEATURES.contains(&feature.as_str()) || !required.insert(feature) {
            bail!("unsupported or duplicate required extension feature {feature:?}");
        }
    }
    if !manifest.dependencies.core
        && !manifest
            .required_features
            .iter()
            .any(|feature| feature == "manifest.dependencies.core.v1")
    {
        bail!("Core-independent extensions require manifest.dependencies.core.v1");
    }
    if !valid_id(&manifest.id) {
        bail!("id must be lowercase latin letters, digits and dashes");
    }
    if manifest.title.trim().is_empty() {
        bail!("title is empty");
    }
    let mut skills = std::collections::HashSet::new();
    let mut paths = std::collections::HashSet::new();
    for instruction in &manifest.agent_instructions {
        if instruction.skill.is_empty()
            || instruction
                .skill
                .chars()
                .any(|ch| ch.is_whitespace() || ch.is_control())
            || !skills.insert(&instruction.skill)
        {
            bail!(
                "agent instruction skill identifiers must be unique and contain no whitespace or control characters"
            );
        }
        let Some(path) = instruction.path.to_str() else {
            bail!("agent instruction paths must be UTF-8 package-relative directories");
        };
        if path.is_empty()
            || path.contains('\\')
            || path.contains(':')
            || path.chars().any(char::is_control)
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
            || !instruction
                .path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)))
            || !paths.insert(&instruction.path)
        {
            bail!(
                "agent instruction paths must be unique package-relative directories without empty, dot, or parent components"
            );
        }
    }
    if let Some(icon) = &manifest.icon
        && super::icons::lucide(icon).is_none()
    {
        bail!("unknown icon {icon:?}; use a Lucide icon file name such as \"folder-tree\"");
    }
    if manifest
        .command
        .first()
        .is_none_or(|program| program.is_empty())
    {
        bail!("command is empty");
    }
    if let Some(tags) = manifest.metadata.0.get(TAGS) {
        let tags = tags
            .as_array()
            .context("org.proteus/tags must be an array of strings")?;
        let mut seen = std::collections::HashSet::new();
        for tag in tags {
            let tag = tag
                .as_str()
                .context("org.proteus/tags entries must be strings")?;
            if tag.is_empty()
                || tag.trim() != tag
                || tag.chars().any(char::is_control)
                || !seen.insert(tag)
            {
                bail!(
                    "org.proteus/tags entries must be unique, non-empty labels without control characters or surrounding whitespace"
                );
            }
        }
    }
    let surfaces = &manifest.surfaces;
    if surfaces.panel.is_none()
        && surfaces.document.is_none()
        && surfaces.badge.is_none()
        && surfaces.page.is_none()
        && surfaces.content.is_none()
        && surfaces.activity.is_none()
        && surfaces.preview.is_none()
    {
        bail!("no surfaces");
    }
    if let Some(content) = &surfaces.content {
        let mut seen = std::collections::HashSet::new();
        if content.languages.is_empty() && content.formats.is_empty() {
            bail!("content requires languages or formats");
        }
        if content
            .languages
            .iter()
            .any(|language| !valid_language(language) || !seen.insert(language))
        {
            bail!("content languages must be unique lowercase fence identifiers");
        }
        let mut seen = std::collections::HashSet::new();
        if content.formats.iter().any(|format| {
            let Some((kind, subtype)) = format.split_once('/') else {
                return true;
            };
            !valid_language(kind) || !valid_language(subtype) || !seen.insert(format)
        }) {
            bail!("content formats must be unique lowercase media types");
        }
    }
    if surfaces.popup.is_some() && surfaces.badge.is_none() && surfaces.activity.is_none() {
        bail!("popup requires a badge or activity");
    }
    if surfaces.activity.is_some() && surfaces.popup.is_none() {
        bail!("activity requires popup content");
    }
    let mut tools = std::collections::HashSet::new();
    if manifest.dependencies.tools.iter().any(|tool| {
        tool.is_empty()
            || tool.trim() != tool
            || tool.chars().any(char::is_control)
            || !tools.insert(tool)
    }) {
        bail!("tool dependencies must be unique non-empty tool names");
    }
    if surfaces
        .popup
        .as_ref()
        .is_some_and(|popup| !popup.width.is_finite() || !(100. ..=1200.).contains(&popup.width))
    {
        bail!("popup width must be in 100..=1200");
    }
    if surfaces
        .popup
        .as_ref()
        .is_some_and(|popup| !popup.height.is_finite() || !(120. ..=1000.).contains(&popup.height))
    {
        bail!("popup height must be in 120..=1000");
    }
    if surfaces
        .badge
        .as_ref()
        .is_some_and(|badge| badge.place.is_rail() && !badge.narrow)
    {
        bail!("a badge with text does not fit the rail");
    }
    Ok(manifest)
}

pub fn valid_language(language: &str) -> bool {
    !language.is_empty()
        && language.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_+.-".contains(&byte)
        })
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !id.starts_with('-')
}

#[cfg(test)]
#[path = "manifest_agent_instructions_tests.rs"]
mod agent_instructions_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_only_subscription_keeps_state_but_rejects_tool_projection() {
        let mut value = serde_json::json!({"protocol":4,"id":"demo","title":"Demo","command":["run"],
            "session_events":true, "surfaces":{"badge":{}}});
        let entries = super::super::session::Update::Entries {
            start: 0,
            removed: 0,
            entries: Vec::new(),
        };
        let state = super::super::session::Update::State {
            state: Box::new(super::super::session::State {
                connected: true,
                config: None,
                context: None,
                execution: Default::default(),
                pending: None,
                activity: None,
            }),
        };
        let manifest = parse(&value.to_string()).unwrap();
        assert!(manifest.accepts_session_update(&entries));
        assert!(manifest.accepts_session_update(&state));
        value["session_entries"] = serde_json::json!(false);
        let manifest = parse(&value.to_string()).unwrap();
        assert!(!manifest.accepts_session_update(&entries));
        assert!(manifest.accepts_session_update(&state));
        value["session_events"] = serde_json::json!(false);
        assert!(
            !parse(&value.to_string())
                .unwrap()
                .accepts_session_update(&state)
        );
    }

    fn write(root: &Path, folder: &str, text: &str) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), text).unwrap();
    }

    #[test]
    fn core_dependency_defaults_to_true_and_local_opt_out_is_explicit() {
        let mut value = serde_json::json!({
            "protocol":4,"id":"local","title":"Local","command":["run"],
            "surfaces":{"document":{}}
        });
        assert!(Dependencies::default().core);
        assert!(parse(&value.to_string()).unwrap().dependencies.core);
        value["dependencies"] = serde_json::json!({"tools":["read_file"]});
        assert!(parse(&value.to_string()).unwrap().dependencies.core);
        value["dependencies"] = serde_json::json!({"core":false});
        assert!(
            parse(&value.to_string()).is_err(),
            "opt-out must name its feature"
        );
        value["required_features"] = serde_json::json!(["manifest.dependencies.core.v1"]);
        assert!(!parse(&value.to_string()).unwrap().dependencies.core);
        for invalid in [
            serde_json::json!(null),
            serde_json::json!("false"),
            serde_json::json!(0),
        ] {
            value["dependencies"]["core"] = invalid;
            assert!(parse(&value.to_string()).is_err());
        }
    }

    #[test]
    fn activity_settings_preview_and_tool_dependencies_are_independent_capabilities() {
        let mut value = serde_json::json!({
            "protocol":4,"id":"plan","title":"Plan","command":["run"],
            "surfaces":{"activity":{},"popup":{},"page":{"embedded":true},"preview":{}},
            "dependencies":{"tools":["update_plan"]}
        });
        let manifest = parse(&value.to_string()).unwrap();
        assert!(manifest.surfaces.badge.is_none());
        assert!(manifest.surfaces.activity.is_some());
        assert!(manifest.surfaces.page.unwrap().embedded);
        assert!(manifest.surfaces.preview.is_some());
        assert_eq!(manifest.dependencies.tools, ["update_plan"]);
        value["surfaces"]["popup"] = serde_json::Value::Null;
        assert!(parse(&value.to_string()).is_err());
        value["surfaces"] = serde_json::json!({"preview":{}});
        assert!(parse(&value.to_string()).is_ok());
        for tools in [
            serde_json::json!([""]),
            serde_json::json!(["x", "x"]),
            serde_json::json!([" x"]),
        ] {
            value["dependencies"]["tools"] = tools;
            assert!(parse(&value.to_string()).is_err());
        }
    }

    #[test]
    fn initial_badge_visibility_is_opt_in_and_tags_are_descriptive() {
        let mut value = serde_json::json!({"protocol":4,"id":"demo","title":"Demo","command":["run"],
            "surfaces":{"badge":{}}, "_meta":{"org.proteus/tags":["Контекстное содержимое"],"org.example/other":{"future":true}}});
        let parsed = parse(&value.to_string()).unwrap();
        assert!(!parsed.surfaces.badge.as_ref().unwrap().initially_hidden);
        assert_eq!(
            parsed.tags().collect::<Vec<_>>(),
            ["Контекстное содержимое"]
        );
        assert_eq!(parsed.required_features, Vec::<String>::new());
        value["surfaces"]["badge"]["initially_hidden"] = serde_json::json!(true);
        value["required_features"] = serde_json::json!(["surfaces.badge.visibility.v1"]);
        assert!(
            parse(&value.to_string())
                .unwrap()
                .surfaces
                .badge
                .unwrap()
                .initially_hidden
        );
        for invalid in [
            serde_json::json!("tag"),
            serde_json::json!([1]),
            serde_json::json!([""]),
            serde_json::json!([" tag"]),
            serde_json::json!(["tag\n"]),
            serde_json::json!(["tag", "tag"]),
        ] {
            value["_meta"]["org.proteus/tags"] = invalid;
            assert!(parse(&value.to_string()).is_err());
        }
    }

    #[test]
    fn discovers_valid_extensions_and_reports_broken_ones() {
        let root = std::env::temp_dir().join(format!("proteus-ext-{}", uuid::Uuid::new_v4()));
        write(
            &root,
            "todo",
            r#"{"protocol": 4, "id": "todo", "title": "Задачи", "command": ["python3", "main.py"],
                "surfaces": {"panel": {"title": "Задачи"}, "badge": {}}}"#,
        );
        write(
            &root,
            "other",
            r#"{"protocol": 4, "id": "todo", "title": "x", "command": ["x"], "surfaces": {"document": {}}}"#,
        );
        write(
            &root,
            "bad",
            r#"{"protocol": 4, "id": "bad", "title": "x", "command": ["x"], "surfaces": {}}"#,
        );
        std::fs::create_dir_all(root.join("empty")).unwrap();

        let (installed, problems) = discover(&root);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(installed.len(), 1);
        let todo = &installed[0].manifest;
        assert_eq!(todo.id, "todo");
        assert_eq!(
            todo.surfaces.badge.as_ref().unwrap().place,
            BadgePlace::ComposerStart
        );
        let broken: Vec<_> = problems
            .iter()
            .map(|problem| problem.dir.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(broken, ["bad", "empty", "other"]);
    }

    #[test]
    fn ids_are_plain_slugs() {
        assert!(valid_id("git-stats2"));
        assert!(!valid_id("Git"));
        assert!(!valid_id("a:b"));
        assert!(!valid_id("-a"));
    }

    #[test]
    fn content_only_packages_register_new_languages_without_host_catalog_entries() {
        let mut value = serde_json::json!({"protocol":4,"id":"figures","title":"Figures",
            "command":["bin/figures"],"required_features":["surfaces.content.v1"],
            "surfaces":{"content":{"languages":["future-diagram","c++"]}}});
        let parsed = parse(&value.to_string()).unwrap();
        assert_eq!(
            parsed.surfaces.content.unwrap().languages,
            ["future-diagram", "c++"]
        );
        for languages in [
            serde_json::json!([]),
            serde_json::json!(["x", "x"]),
            serde_json::json!(["UpperCase"]),
            serde_json::json!(["two words"]),
        ] {
            value["surfaces"]["content"]["languages"] = languages;
            assert!(parse(&value.to_string()).is_err());
        }
    }

    #[test]
    fn content_formats_are_media_types_and_need_no_fence_registration() {
        let mut value = serde_json::json!({"protocol":4,"id":"text","title":"Text",
            "command":["bin/text"], "surfaces":{"content":{"formats":["text/markdown"]}}});
        let parsed = parse(&value.to_string()).unwrap();
        assert_eq!(parsed.surfaces.content.unwrap().formats, ["text/markdown"]);
        for formats in [
            serde_json::json!([]),
            serde_json::json!(["markdown"]),
            serde_json::json!(["text/markdown", "text/markdown"]),
            serde_json::json!(["TEXT/markdown"]),
            serde_json::json!(["text/markdown/extra"]),
        ] {
            value["surfaces"]["content"]["formats"] = formats;
            assert!(parse(&value.to_string()).is_err());
        }
    }

    #[test]
    fn metadata_is_optional_but_required_features_and_behavior_are_checked() {
        let mut value = serde_json::json!({"protocol":4,"id":"demo","title":"Demo","command":["run"],
            "required_features":["session.settings.v1"], "surfaces":{"page":{}},
            "_meta":{"org.example/catalog":{"version":"future"}}});
        parse(&value.to_string()).unwrap();
        value["required_features"] = serde_json::json!(["future.required.v1"]);
        assert!(
            parse(&value.to_string())
                .unwrap_err()
                .to_string()
                .contains("required extension feature")
        );
        value["required_features"] = serde_json::json!([]);
        value["unknown_behavior"] = serde_json::json!(true);
        assert!(parse(&value.to_string()).is_err());
    }

    #[test]
    fn user_package_overrides_bundled_and_broken_update_stays_visible() {
        let root = std::env::temp_dir().join(format!("proteus-sources-{}", uuid::Uuid::new_v4()));
        let bundled = root.join("bundled");
        let user = root.join("user");
        let text = r#"{"protocol":4,"id":"quota","title":"Quota","command":["python3","main.py"],"surfaces":{"badge":{"narrow":true,"place":"rail_end"}}}"#;
        write(&bundled, "quota", text);
        write(&user, "quota", &text.replace("Quota", "Updated"));
        let (found, errors) = discover_sources(&bundled, Some(&user));
        assert!(errors.is_empty());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.title, "Updated");
        write(
            &user,
            "quota",
            &text.replace("\"protocol\":4", "\"protocol\":1"),
        );
        let (found, errors) = discover_sources(&bundled, Some(&user));
        assert!(found.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].error.contains("unsupported extension protocol"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
