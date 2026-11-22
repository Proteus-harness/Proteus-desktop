//! Module-owned connection templates. Discovery runs on a worker; rendering
//! receives immutable choices and never guesses provider implementations.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
};

use anyhow::{Context, Result, ensure};
use proteus_contracts::app_protocol::{
    config_builder::ConfigBuilderSnapshot,
    http::{ComponentPackageInstall, SetConfigBuilderRequest},
};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Installed, Target, inventory::Inventory, schema};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    pub id: String,
    pub title: String,
    pub description: String,
    pub module_id: String,
    pub module_config: Value,
    pub env_allowlist: Vec<String>,
    #[serde(default)]
    pub auth: Option<Authentication>,
}

/// Management argv owned by the installed module, outside the model protocol.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authentication {
    pub command: Vec<String>,
    pub credential_file_key: String,
}

impl Authentication {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.command.is_empty()
                && self.command.len() <= 16
                && self.command.iter().all(|arg| {
                    !arg.is_empty() && arg.len() <= 1024 && !arg.chars().any(char::is_control)
                })
                && !self.credential_file_key.is_empty()
                && !self.credential_file_key.chars().any(char::is_control),
            "Неверная команда авторизации модуля"
        );
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModuleManifest {
    pub id: String,
    pub version: String,
    pub command: Vec<String>,
    pub exports: BTreeMap<String, Vec<String>>,
    pub sha256: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub model_connections: Vec<Template>,
}

impl ModuleManifest {
    pub(crate) fn validate_connections(&self) -> Result<()> {
        let mut seen = BTreeSet::new();
        for template in &self.model_connections {
            identity(&template.id)?;
            identity(&template.module_id)?;
            ensure!(
                seen.insert(&template.id),
                "Повтор варианта подключения: {}",
                template.id
            );
            if let Some(auth) = &template.auth {
                auth.validate()?;
            }
            ensure!(
                !template.title.trim().is_empty(),
                "У варианта подключения нет названия"
            );
            ensure!(
                template.module_config.is_object(),
                "Параметры подключения должны быть объектом"
            );
            ensure!(
                self.exports
                    .get("model")
                    .is_some_and(|ids| ids.iter().any(|id| id == "*" || id == &template.module_id)),
                "Вариант подключения не объявлен в экспортах пакета"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Choice {
    pub package: String,
    pub command: Vec<String>,
    pub icon: Option<String>,
    pub template: Template,
    /// Whether the package can instantiate a caller-named model export.
    pub named: bool,
}

impl Choice {
    pub fn key(&self) -> String {
        super::key(&format!("{:?}\n{}", self.command, self.template.id))
    }

    pub fn available_id(&self, snapshot: &ConfigBuilderSnapshot) -> String {
        let base = &self.template.module_id;
        if !self.named {
            return base.clone();
        }
        (1..)
            .map(|number| {
                if number == 1 {
                    base.clone()
                } else {
                    format!("{base}-{number}")
                }
            })
            .find(|id| self.check_id(id, snapshot).is_ok())
            .expect("unbounded unique connection id")
    }

    pub fn check_id(&self, id: &str, snapshot: &ConfigBuilderSnapshot) -> Result<()> {
        identity(id)?;
        ensure!(
            self.named || id == self.template.module_id,
            "Этот модуль использует фиксированное имя подключения"
        );
        ensure!(
            !connection_exists(snapshot, id),
            "Подключение {id} уже есть в конфиге"
        );
        ensure!(
            !snapshot
                .module_config
                .get("model")
                .is_some_and(|config| config.contains_key(id)),
            "Параметры с именем {id} уже есть в конфиге"
        );
        Ok(())
    }

    pub fn request(
        &self,
        id: &str,
        snapshot: &ConfigBuilderSnapshot,
    ) -> Result<SetConfigBuilderRequest> {
        self.check_id(id, snapshot)?;
        let base = format!("model-{id}");
        let component = (1..)
            .map(|number| {
                if number == 1 {
                    base.clone()
                } else {
                    format!("{base}-{number}")
                }
            })
            .find(|id| !snapshot.plugins.iter().any(|plugin| &plugin.id == id))
            .expect("unbounded unique component id");
        Ok(SetConfigBuilderRequest {
            component_packages: vec![ComponentPackageInstall {
                id: component,
                command: self.command.clone(),
                icon: self.icon.clone(),
                exports: BTreeMap::from([("model".into(), vec![id.to_owned()])]),
                env_allowlist: self.template.env_allowlist.clone(),
                initial_config: BTreeMap::from([(
                    "model".into(),
                    BTreeMap::from([(id.to_owned(), self.template.module_config.clone())]),
                )]),
                replace: Vec::new(),
            }],
            ..Default::default()
        })
    }
}

fn identity(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.trim() == id && id != "*" && !id.chars().any(char::is_control),
        "Укажите непустое имя подключения без управляющих символов"
    );
    Ok(())
}

pub fn connection_exists(snapshot: &ConfigBuilderSnapshot, id: &str) -> bool {
    snapshot.model_modules.iter().any(|module| module.id == id)
        || snapshot
            .plugins
            .iter()
            .flat_map(|plugin| &plugin.exports)
            .any(|export| export.slot == "model" && export.id == id)
}

#[derive(Default)]
pub struct Discovery {
    pub choices: Vec<Choice>,
    pub problems: Vec<String>,
}

/// Bundled and already installed packages share one metadata format. No
/// provider calls, key reads or package downloads are part of discovery.
pub fn discover(installed: &[Installed]) -> Discovery {
    discover_with_inventory(installed, Inventory::local_modules())
}

fn discover_with_inventory(installed: &[Installed], inventory: Inventory) -> Discovery {
    let mut paths = BTreeMap::new();
    for (id, package) in inventory.module_packages() {
        if let Some(root) = package
            .path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
        {
            paths.insert(
                package.path.join("module.json"),
                (root.to_path_buf(), id.to_owned()),
            );
        }
    }
    for entry in installed {
        if let Target::Component { path, .. } = &entry.target {
            paths.insert(
                path.join("share/modules")
                    .join(&entry.entry)
                    .join("module.json"),
                (path.clone(), entry.entry.clone()),
            );
        }
    }
    let mut result = Discovery::default();
    let mut seen = BTreeSet::new();
    for (manifest, (root, id)) in paths {
        match read_choices(&manifest, &root) {
            Ok(choices) => {
                for choice in choices {
                    if seen.insert(choice.key()) {
                        result.choices.push(choice);
                    }
                }
            }
            Err(error) => result.problems.push(format!("{id}: {error:#}")),
        }
    }
    result.choices.sort_by(|a, b| {
        a.template
            .title
            .cmp(&b.template.title)
            .then(a.package.cmp(&b.package))
    });
    result
}

fn read_choices(manifest_path: &Path, root: &Path) -> Result<Vec<Choice>> {
    let metadata = match std::fs::metadata(manifest_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.len() <= 128 * 1024,
        "module.json превышает 128 КиБ"
    );
    let manifest: ModuleManifest =
        serde_json::from_slice(&std::fs::read(manifest_path)?).context("Неверный module.json")?;
    if manifest.model_connections.is_empty() {
        return Ok(Vec::new());
    }
    manifest.validate_connections()?;
    let first = manifest.command.first().context("У модуля нет команды")?;
    schema::relative(Path::new(first))?;
    let executable = root.join(first);
    let mut file = std::fs::File::open(&executable)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    ensure!(
        format!("{:x}", hash.finalize()).eq_ignore_ascii_case(&manifest.sha256),
        "Исполняемый файл не соответствует module.json"
    );
    let mut command = manifest.command;
    command[0] = executable
        .to_str()
        .context("Путь модуля не в UTF-8")?
        .to_owned();
    let named = manifest
        .exports
        .get("model")
        .is_some_and(|ids| ids.iter().any(|id| id == "*"));
    let icon = module_icon(
        manifest.icon.as_deref(),
        manifest_path.parent().context("У модуля нет папки")?,
    );
    Ok(manifest
        .model_connections
        .into_iter()
        .map(|template| Choice {
            package: manifest.id.clone(),
            command: command.clone(),
            icon: icon.clone(),
            template,
            named,
        })
        .collect())
}

/// Relative author artwork belongs to the module package. Persist its stable
/// location with the executable; plain Lucide names and bundled assets stay IDs.
pub(crate) fn module_icon(value: Option<&str>, directory: &Path) -> Option<String> {
    let value = value.filter(|value| !value.is_empty())?;
    let path = Path::new(value);
    if !path.is_absolute()
        && !value.contains(':')
        && (value.contains('/') || value.ends_with(".svg") || value.ends_with(".png"))
    {
        Some(directory.join(path).to_string_lossy().into_owned())
    } else {
        Some(value.to_owned())
    }
}

#[cfg(test)]
mod tests;
