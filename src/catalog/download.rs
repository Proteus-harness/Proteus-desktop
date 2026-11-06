//! A source download is a new, immutable folder with a complete build workspace
//! and instructions beside it. It never creates an installation receipt.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use super::{
    Locations, Source, git,
    schema::{Catalog, Entry},
};

pub fn source(
    locations: &Locations,
    repository: &Source,
    catalog: &Catalog,
    entry: &Entry,
    parent: &Path,
) -> Result<PathBuf> {
    // Resolve metadata and revision from the same snapshot, including a click
    // delivered from an older rendered frame after a source refresh.
    let entry = catalog
        .entries
        .iter()
        .find(|current| current.id == entry.id && current.kind == entry.kind)
        .context("Запись изменилась; обновите выбор в каталоге")?;
    let package = entry.source.as_ref().context("У записи нет исходников")?;
    let public_repository = url::Url::parse(&repository.url)
        .map(|mut url| {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        })
        .unwrap_or_else(|_| repository.name());
    let revision = catalog
        .revision
        .as_deref()
        .context("Обновите Git-источник перед скачиванием")?;
    ensure!(
        (revision.len() == 40 || revision.len() == 64)
            && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Неверная ревизия источника"
    );
    let parent = parent.canonicalize()?;
    ensure!(parent.is_dir(), "Выберите папку для скачивания");
    let destination = parent.join(format!("{}-{}", entry.id, &revision[..12]));
    ensure!(
        !destination.is_symlink() && !destination.try_exists()?,
        "Папка уже существует: {}",
        destination.display()
    );
    let staging = parent.join(format!(".proteus-source-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging)?;
    let result = (|| {
        git::export_source(
            &locations.cache,
            repository,
            revision,
            &package.path,
            &staging.join("source"),
        )?;
        let requirements = if package.requirements.is_empty() {
            String::new()
        } else {
            format!("Нужны: {}.\n\n", package.requirements.join(", "))
        };
        let instructions = format!(
            "# {}\n\nИсходники: {}\nРевизия: {}\nПакет: {} ({})\n\n{}Из этой папки:\n\n```sh\ncd source\n{}\n```\n\nКлиент не запускал сборку и не подключал пакет к профилю.\n",
            entry.title,
            public_repository,
            revision,
            entry.id,
            entry.version,
            requirements,
            package.command(),
        );
        std::fs::write(staging.join("BUILD.md"), instructions)?;
        std::fs::write(
            staging.join("source.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "repository": {"url": public_repository, "branch": repository.branch, "key": repository.key()}, "revision": revision, "entry": entry.id,
                "version": entry.version, "kind": entry.kind, "build": package.build,
            }))?,
        )?;
        crate::packages::publish(&staging, &destination, false)?;
        Ok(destination)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}
