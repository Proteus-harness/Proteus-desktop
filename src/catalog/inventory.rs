//! Packages already present outside catalog receipts. Read once on a worker;
//! rendering only looks up the small immutable inventory.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::{
    Source,
    open::OFFICIAL,
    schema::{self, Kind},
};

#[derive(Clone, Debug)]
pub struct Package {
    /// Older extension manifests do not declare a version.
    pub version: Option<String>,
    pub path: PathBuf,
    pub problem: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Inventory {
    modules: BTreeMap<String, Package>,
    extensions: BTreeMap<String, Package>,
}

impl Inventory {
    pub fn local() -> Self {
        let backend = resolve_binary(&crate::backend::proteus_binary());
        let bundled = crate::declarative::manifest::bundled_root();
        let user = crate::declarative::manifest::root();
        Self::read(backend.as_deref(), &bundled, user.as_deref())
    }

    pub fn read(backend: Option<&Path>, bundled: &Path, user: Option<&Path>) -> Self {
        let mut inventory = Self::default();
        if let Some(backend) = backend {
            inventory.read_modules(backend);
        }
        let (extensions, problems) = crate::declarative::manifest::discover_sources(bundled, user);
        for installed in extensions {
            inventory.extensions.insert(
                installed.manifest.id,
                Package {
                    version: None,
                    path: installed.dir,
                    problem: None,
                },
            );
        }
        // A broken user override is still an installation; never silently
        // report the bundled package with the same id as healthy.
        for problem in problems {
            if let Some(id) = problem.dir.file_name().and_then(|id| id.to_str())
                && schema::slug(id)
                && (problem.dir.parent() == user || !inventory.extensions.contains_key(id))
            {
                inventory.extensions.insert(
                    id.to_owned(),
                    Package {
                        version: None,
                        path: problem.dir,
                        problem: Some(problem.error),
                    },
                );
            }
        }
        inventory
    }

    /// Matching ids in an unrelated repository do not establish provenance.
    /// Receipted third-party packages are matched by the caller before this.
    pub fn find(&self, source: &Source, kind: Kind, id: &str) -> Option<&Package> {
        match kind {
            Kind::ProcessModule if OFFICIAL[0].matches(source) => self.modules.get(id),
            Kind::ClientExtension if OFFICIAL[1].matches(source) => self.extensions.get(id),
            _ => None,
        }
    }

    fn read_modules(&mut self, backend: &Path) {
        let Some(bin) = backend.parent() else { return };
        let root = if bin.file_name().is_some_and(|name| name == "bin") {
            bin.parent().unwrap_or(bin)
        } else {
            bin
        };
        let Ok(entries) = std::fs::read_dir(root.join("share/modules")) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(id) = path
                .file_name()
                .and_then(|id| id.to_str())
                .filter(|id| schema::slug(id))
            else {
                continue;
            };
            let manifest = path.join("module.json");
            let parsed = (|| -> anyhow::Result<Module> {
                anyhow::ensure!(
                    std::fs::metadata(&manifest)?.len() <= 128 * 1024,
                    "module.json превышает 128 КиБ"
                );
                let module: Module = serde_json::from_slice(&std::fs::read(manifest)?)?;
                anyhow::ensure!(
                    module.id == id && !module.version.is_empty(),
                    "Неверные id или version модуля"
                );
                let command = module
                    .command
                    .first()
                    .ok_or_else(|| anyhow::anyhow!("Нет команды модуля"))?;
                let executable = Path::new(command);
                schema::relative(executable)?;
                anyhow::ensure!(
                    root.join(executable).is_file(),
                    "Не найден исполняемый файл модуля: {command}"
                );
                Ok(module)
            })();
            let (version, problem) = match parsed {
                Ok(module) => (Some(module.version), None),
                Err(error) => (None, Some(format!("{error:#}"))),
            };
            self.modules.insert(
                id.to_owned(),
                Package {
                    version,
                    path,
                    problem,
                },
            );
        }
    }
}

#[derive(Deserialize)]
struct Module {
    id: String,
    version: String,
    command: Vec<String>,
}

fn resolve_binary(binary: &Path) -> Option<PathBuf> {
    if binary.components().count() > 1 || binary.is_absolute() {
        return std::fs::canonicalize(binary).ok();
    }
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|directory| {
        let path = directory.join(binary);
        path.is_file()
            .then(|| std::fs::canonicalize(path).ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests;
