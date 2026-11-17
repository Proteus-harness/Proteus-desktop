//! Lazy directory listing of user-owned agent configurations. Discovery reads
//! directory entries and optional profile labels: loading and validation happen
//! after a choice. No includes, providers or model catalogs are resolved here.

use std::collections::{HashMap, HashSet};
use std::io::Read as _;
use std::path::{Path, PathBuf};

const MAX_DIRECTORIES: usize = 256;
const MAX_CONFIGS: usize = 1024;
const MAX_DEPTH: usize = 5;
const MAX_ENTRIES: usize = 8192;
const MAX_METADATA_BYTES: usize = 256 * 1024;
const MAX_METADATA_TOTAL: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConfigFile {
    pub path: PathBuf,
    pub relative: String,
    pub name: String,
    pub current: bool,
    pub title: Option<String>,
    pub entrypoint: bool,
    pub duplicate_name: bool,
}

impl ConfigFile {
    pub fn display_name(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct Metadata {
    profile: ProfileMetadata,
}

#[derive(serde::Deserialize)]
#[serde(default)]
struct ProfileMetadata {
    title: Option<String>,
    entrypoint: bool,
}

impl Default for ProfileMetadata {
    fn default() -> Self {
        Self {
            title: None,
            entrypoint: true,
        }
    }
}

#[derive(Default)]
pub(super) struct Catalog {
    pub files: Vec<ConfigFile>,
    pub problem: Option<String>,
}

pub(super) fn config_home() -> Option<PathBuf> {
    home_from(
        std::env::var_os("PROTEUS_CONFIG_HOME"),
        std::env::var_os("HOME"),
        std::env::var_os("XDG_CONFIG_HOME"),
    )
}

fn home_from(
    configured: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
    xdg: Option<std::ffi::OsString>,
) -> Option<PathBuf> {
    configured
        .map(PathBuf::from)
        .or_else(|| home.map(|home| PathBuf::from(home).join(".config/Proteus-harness")))
        .or_else(|| xdg.map(|xdg| PathBuf::from(xdg).join("Proteus-harness")))
}

pub(super) fn discover(root: Option<PathBuf>, current: Option<PathBuf>) -> Catalog {
    let Some(root) = root else {
        return Catalog {
            problem: Some("Папка конфигов не определена".into()),
            ..Catalog::default()
        };
    };
    let current = current.and_then(|path| path.canonicalize().ok());
    let mut catalog = Catalog::default();
    let mut directories = HashSet::new();
    let mut files = HashSet::new();
    let mut entries = 0;
    let mut metadata_bytes = 0;
    scan(
        &root,
        &root,
        0,
        &current,
        &mut directories,
        &mut files,
        &mut entries,
        &mut metadata_bytes,
        &mut catalog,
    );
    let mut names = HashMap::new();
    for file in &catalog.files {
        *names.entry(file.display_name().to_owned()).or_insert(0) += 1;
    }
    for file in &mut catalog.files {
        file.duplicate_name = names[file.display_name()] > 1;
    }
    catalog.files.sort_by(|a, b| {
        a.display_name()
            .to_lowercase()
            .cmp(&b.display_name().to_lowercase())
            .then_with(|| a.relative.cmp(&b.relative))
    });
    catalog
}

fn scan(
    root: &Path,
    directory: &Path,
    depth: usize,
    current: &Option<PathBuf>,
    directories: &mut HashSet<PathBuf>,
    files: &mut HashSet<PathBuf>,
    visited_entries: &mut usize,
    metadata_bytes: &mut usize,
    catalog: &mut Catalog,
) {
    let physical = match directory.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && depth == 0 => return,
        Err(error) => {
            catalog.problem.get_or_insert_with(|| {
                format!("Не удалось прочитать {}: {error}", directory.display())
            });
            return;
        }
    };
    if directories.contains(&physical) {
        return;
    }
    if directories.len() >= MAX_DIRECTORIES || catalog.files.len() >= MAX_CONFIGS {
        catalog
            .problem
            .get_or_insert_with(|| "Список ограничен; другой файл можно выбрать вручную".into());
        return;
    }
    directories.insert(physical);
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            catalog.problem.get_or_insert_with(|| {
                format!("Не удалось прочитать {}: {error}", directory.display())
            });
            return;
        }
    };
    let mut paths = Vec::new();
    for entry in entries {
        if *visited_entries >= MAX_ENTRIES {
            catalog.problem.get_or_insert_with(|| {
                "Список ограничен; другой файл можно выбрать вручную".into()
            });
            break;
        }
        *visited_entries += 1;
        match entry {
            Ok(entry) => paths.push(entry.path()),
            Err(error) => {
                catalog
                    .problem
                    .get_or_insert_with(|| format!("Не удалось прочитать список: {error}"));
            }
        }
    }
    paths.sort();
    let profiles = depth == 0 || directory.file_name().is_some_and(|name| name == "configs");
    for path in paths {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if ignored(name) {
            continue;
        }
        if path.is_dir() {
            // A configs directory contains top-level profiles only. Fragments,
            // prompts and other implementation folders are never a menu tree.
            if profiles && depth > 0 {
                continue;
            }
            if depth < MAX_DEPTH {
                scan(
                    root,
                    &path,
                    depth + 1,
                    current,
                    directories,
                    files,
                    visited_entries,
                    metadata_bytes,
                    catalog,
                );
            } else {
                catalog.problem.get_or_insert_with(|| {
                    "Глубина списка ограничена; другой файл можно выбрать вручную".into()
                });
            }
        } else if profiles && config_file(&path) && path.is_file() {
            let Ok(physical) = path.canonicalize() else {
                continue;
            };
            if !files.insert(physical.clone()) {
                continue;
            }
            let metadata = profile_metadata(&path, metadata_bytes, &mut catalog.problem);
            catalog.files.push(ConfigFile {
                relative: path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
                name: name.to_owned(),
                current: current.as_ref() == Some(&physical),
                title: metadata.title,
                entrypoint: metadata.entrypoint,
                duplicate_name: false,
                path,
            });
            if catalog.files.len() >= MAX_CONFIGS {
                catalog.problem.get_or_insert_with(|| {
                    "Список ограничен; другой файл можно выбрать вручную".into()
                });
                break;
            }
        }
    }
}

/// Deserialize only the launcher's presentation keys. Invalid or oversized
/// documents remain ordinary visible entries, rather than disappearing because
/// discovery tried to validate an agent configuration.
fn profile_metadata(
    path: &Path,
    bytes: &mut usize,
    problem: &mut Option<String>,
) -> ProfileMetadata {
    let limit = MAX_METADATA_BYTES.min(MAX_METADATA_TOTAL.saturating_sub(*bytes));
    if limit == 0 {
        problem
            .get_or_insert_with(|| "Часть подписей не прочитана; файлы доступны в списке".into());
        return ProfileMetadata::default();
    }
    let Some(mut file) = std::fs::File::open(path)
        .ok()
        .map(|file| file.take((limit + 1) as u64))
    else {
        return ProfileMetadata::default();
    };
    let mut data = Vec::new();
    if file.read_to_end(&mut data).is_err() {
        return ProfileMetadata::default();
    }
    *bytes += data.len();
    if data.len() > limit {
        problem
            .get_or_insert_with(|| "Часть подписей не прочитана; файлы доступны в списке".into());
        return ProfileMetadata::default();
    }
    let Ok(source) = std::str::from_utf8(&data) else {
        return ProfileMetadata::default();
    };
    let parsed = if path
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        serde_json::from_str::<Metadata>(source).ok()
    } else {
        toml::from_str::<Metadata>(source).ok()
    };
    let mut profile = parsed.unwrap_or_default().profile;
    profile.title = profile
        .title
        .map(|title| title.trim().to_owned())
        .filter(|title| !title.is_empty());
    profile
}

fn config_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("toml" | "json")
    ) && path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| !name.contains(".example.") && !name.starts_with("example."))
}

fn ignored(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with("configs-backup-")
        || name.starts_with("configs.broken-link-")
        || matches!(
            name,
            "secrets"
                | "sessions"
                | "fragments"
                | "prompts"
                | "cache"
                | "runtime"
                | "target"
                | "node_modules"
                | "backups"
                | "archive"
                | "configs.bak"
                | "config-history"
                | "model-selections"
                | "logs"
                | "sdk"
                | "skills"
                | "themes"
        )
}

#[cfg(test)]
mod tests;
