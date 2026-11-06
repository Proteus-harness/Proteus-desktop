//! Installation identity and profile binding shared by catalog and ZIP updates.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use proteus_contracts::domain::AgentPluginConfig;

use crate::{catalog::Locations, packages::Contents, settings::Draft};

/// Prepared files and the exact package list against which a replacement was
/// offered. A changed draft must be reviewed again instead of overwriting it.
pub struct ZipImport {
    pub path: PathBuf,
    pub name: String,
    pub previous: Option<AgentPluginConfig>,
    packages: Vec<AgentPluginConfig>,
    binding: AgentPluginConfig,
}

impl ZipImport {
    pub fn connect(mut self, draft: &mut Draft) -> Result<()> {
        ensure!(
            draft.addons.addons.plugins == self.packages,
            "Список пакетов изменился во время импорта; выберите ZIP ещё раз"
        );
        // Bind the path actually returned by filesystem publication.
        self.binding.path = self.path;
        connect(draft, self.binding, self.previous.as_ref());
        Ok(())
    }
}

/// Runs on a worker thread. Preparing an archive never changes a profile or
/// adopts an existing package merely because its manifest has the same name.
pub fn import_zip(
    locations: &Locations,
    path: &Path,
    packages: Vec<AgentPluginConfig>,
    cwd: Option<&Path>,
) -> Result<ZipImport> {
    let files = Contents::zip(path)?;
    let name = name(files.text("plugin.json")?)?;
    let mut previous = None;
    let mut previous_root = None;
    for package in &packages {
        let root = resolve_root(&package.path, cwd)?;
        let current_name = read_name(&root).with_context(|| {
            format!(
                "Не удалось проверить имя подключённого пакета {}",
                package.path.display()
            )
        })?;
        if current_name == name {
            ensure!(
                previous.is_none(),
                "В профиле несколько пакетов «{name}»; сначала уберите лишние подключения"
            );
            previous = Some(package.clone());
            previous_root = Some(root);
        }
    }
    // Resolve the previous address before publishing the new immutable files.
    let id = format!("zip-{}", uuid::Uuid::new_v4());
    let binding = binding_from(
        locations.plugins.join(&id),
        previous.as_ref(),
        previous_root.as_deref(),
    )?;
    let path = files.install(&locations.plugins, &id, false)?;
    Ok(ZipImport {
        path,
        name,
        previous,
        packages,
        binding,
    })
}

/// Preserve the state address before changing an immutable package root.
/// The root-based default remains the address for an unchanged binding.
pub(super) fn binding(
    path: PathBuf,
    previous: Option<&AgentPluginConfig>,
) -> Result<AgentPluginConfig> {
    binding_from(path, previous, None)
}

fn binding_from(
    path: PathBuf,
    previous: Option<&AgentPluginConfig>,
    previous_root: Option<&Path>,
) -> Result<AgentPluginConfig> {
    let Some(previous) = previous else {
        return Ok(AgentPluginConfig {
            path,
            enabled: true,
            data_key: None,
        });
    };
    let data_key = if previous.path == path || previous.data_key.is_some() {
        previous.data_key.clone()
    } else {
        let root = previous_root
            .unwrap_or(&previous.path)
            .canonicalize()
            .with_context(|| {
                format!(
                    "Не удалось сохранить адрес данных прежнего пакета {}",
                    previous.path.display()
                )
            })?;
        Some(root.to_string_lossy().into_owned())
    };
    Ok(AgentPluginConfig {
        path,
        enabled: previous.enabled,
        data_key,
    })
}

/// Resolve the documented Core path shorthands in the same local environment;
/// a relative path belongs to the bound Core workspace, never Desktop's cwd.
fn resolve_root(path: &Path, cwd: Option<&Path>) -> Result<PathBuf> {
    let mut expanded = path.to_owned();
    if let Some(text) = path.to_str() {
        for home in ["~", "$HOME", "${HOME}"] {
            let tail = if text == home {
                Some("")
            } else {
                text.strip_prefix(home)
                    .and_then(|tail| tail.strip_prefix('/'))
            };
            if let Some(tail) = tail {
                expanded =
                    PathBuf::from(std::env::var_os("HOME").context("Не задан HOME")?).join(tail);
                break;
            }
        }
    }
    if expanded.is_absolute() {
        return Ok(expanded);
    }
    let cwd = cwd.context("Для относительной папки пакета нужен рабочий каталог Core")?;
    ensure!(
        cwd.is_absolute(),
        "Рабочий каталог Core должен быть абсолютным"
    );
    Ok(cwd.join(expanded))
}

pub(super) fn connect(
    draft: &mut Draft,
    binding: AgentPluginConfig,
    previous: Option<&AgentPluginConfig>,
) {
    let packages = &mut draft.addons.addons.plugins;
    if let Some(ix) = previous.and_then(|previous| {
        packages
            .iter()
            .position(|plugin| plugin.path == previous.path)
    }) {
        packages[ix] = binding;
    } else if let Some(ix) = packages
        .iter()
        .position(|plugin| plugin.path == binding.path)
    {
        packages[ix] = binding;
    } else {
        packages.push(binding);
    }
}

fn read_name(root: &Path) -> Result<String> {
    const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
    let mut text = String::new();
    File::open(root.join("plugin.json"))?
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)?;
    ensure!(
        text.len() as u64 <= MAX_MANIFEST_BYTES,
        "Слишком большой plugin.json"
    );
    name(&text)
}

/// Core remains the authoritative Agent Plugins validator; installation checks
/// the required manifest identity before writing any managed files.
pub(super) fn name(text: &str) -> Result<String> {
    let value: serde_json::Value = serde_json::from_str(text)?;
    ensure!(
        value["$schema"] == "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
        "Нужен Agent Plugins 1.0 plugin.json"
    );
    let name = value["name"].as_str().context("В plugin.json нужен name")?;
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && !name.contains("..")
            && !name.contains("--")
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
            && name.as_bytes()[0].is_ascii_alphanumeric()
            && name.as_bytes()[name.len() - 1].is_ascii_alphanumeric(),
        "Неверное имя Agent Plugin"
    );
    Ok(name.to_owned())
}
