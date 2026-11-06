//! Source downloads are immutable Git snapshots, not installed packages.
//! Read blob bytes directly: checkout attributes and filters must not alter
//! the sources or run a command while the user only downloads them.

use std::{
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Component, Path},
};

use anyhow::{Context, Result, ensure};

use super::{Item, Source, fetch_files, list, lock, repo_path, run, run_to_file, schema};

const MAX_FILES: usize = 16_384;
const MAX_BYTES: u64 = 256 * 1024 * 1024;

/// Write the selected revision into a new directory, retaining workspace
/// dependencies, executable modes and symlinks. Nothing is built or installed.
pub fn export_source(
    cache: &Path,
    source: &Source,
    revision: &str,
    root: &Path,
    destination: &Path,
) -> Result<()> {
    export_bounded(
        cache,
        source,
        revision,
        root,
        destination,
        MAX_FILES,
        MAX_BYTES,
    )
}

fn export_bounded(
    cache: &Path,
    source: &Source,
    revision: &str,
    root: &Path,
    destination: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<()> {
    ensure!(
        matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Неверная Git-ревизия исходников"
    );
    schema::relative(root)?;
    let root = root
        .components()
        .filter(|part| !matches!(part, Component::CurDir))
        .collect::<std::path::PathBuf>();
    let root_text = root.to_str().context("Путь исходников не в UTF-8")?;
    match fs::symlink_metadata(destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
        Ok(_) => anyhow::bail!("Папка уже существует: {}", destination.display()),
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    ensure!(
        parent.is_dir(),
        "Выберите существующую папку для исходников"
    );
    let lock = lock(source)?;
    let _guard = lock
        .lock()
        .map_err(|_| anyhow::anyhow!("Предыдущее обновление каталога прервано"))?;
    let repo = repo_path(cache, source);
    let repo = repo.to_str().context("Путь к кэшу не в UTF-8")?;
    let tree = if root_text.is_empty() {
        format!("{revision}^{{tree}}")
    } else {
        format!("{revision}:{root_text}")
    };
    ensure!(
        run(&["--git-dir", repo, "cat-file", "-t", &tree], cache, None)? == b"tree\n",
        "В выбранной ревизии нужна папка исходников"
    );
    let paths = if root_text.is_empty() {
        Vec::new()
    } else {
        vec![root_text]
    };
    let items = list(repo, revision, &paths, false, cache)?;
    ensure!(!items.is_empty(), "Папка исходников пуста");
    ensure!(
        items.len() <= max_files,
        "В исходниках больше {max_files} файлов"
    );
    for item in &items {
        schema::relative(Path::new(&item.path))?;
        ensure!(
            item.kind == "blob" && matches!(item.mode.as_str(), "100644" | "100755" | "120000"),
            "Не поддерживается объект исходников {}: {}",
            item.kind,
            item.path
        );
        Path::new(&item.path)
            .strip_prefix(&root)
            .context("Файл находится вне папки исходников")?;
    }
    let files = items.iter().collect::<Vec<_>>();
    fetch_files(repo, &files, cache)?;
    let input = files
        .iter()
        .map(|item| format!("{}\n", item.oid))
        .collect::<String>();
    let metadata = run(
        &["--git-dir", repo, "cat-file", "--batch-check"],
        cache,
        Some(input.as_bytes()),
    )?;
    let metadata = String::from_utf8(metadata).context("Неверные размеры Git-объектов")?;
    let sizes = sizes(&metadata, &files, max_bytes)?;

    let staging = parent.join(format!(".source-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&staging)?;
    let result = (|| {
        let payload = staging.join("objects");
        run_to_file(
            &["--git-dir", repo, "cat-file", "--batch"],
            cache,
            Some(input.as_bytes()),
            &payload,
        )?;
        let snapshot = staging.join("snapshot");
        fs::create_dir(&snapshot)?;
        let mut input = BufReader::new(File::open(&payload)?);
        for (item, size) in files.iter().zip(sizes) {
            let mut header = String::new();
            input.by_ref().take(256).read_line(&mut header)?;
            ensure!(
                header == format!("{} blob {size}\n", item.oid),
                "Неожиданный Git-объект исходников"
            );
            let target = snapshot.join(Path::new(&item.path).strip_prefix(&root)?);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            if item.mode == "120000" {
                let mut value = Vec::new();
                input.by_ref().take(size).read_to_end(&mut value)?;
                ensure!(value.len() as u64 == size, "Git оборвал содержимое ссылки");
                use std::os::unix::ffi::OsStringExt;
                symlink(std::ffi::OsString::from_vec(value), target)?;
            } else {
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                ensure!(
                    std::io::copy(&mut input.by_ref().take(size), &mut output)? == size,
                    "Git оборвал содержимое файла"
                );
                let mode = if item.mode == "100755" { 0o755 } else { 0o644 };
                fs::set_permissions(target, fs::Permissions::from_mode(mode))?;
            }
            let mut separator = [0];
            input.read_exact(&mut separator)?;
            ensure!(separator == [b'\n'], "Неверная граница Git-объекта");
        }
        ensure!(
            input.fill_buf()?.is_empty(),
            "Лишние Git-объекты исходников"
        );
        crate::packages::publish(&snapshot, destination, false)
    })();
    let _ = fs::remove_dir_all(staging);
    result
}

fn sizes(metadata: &str, files: &[&Item], max_bytes: u64) -> Result<Vec<u64>> {
    let mut total = 0u64;
    let mut sizes = Vec::with_capacity(files.len());
    for (line, file) in metadata.lines().zip(files) {
        let mut fields = line.split(' ');
        ensure!(
            fields.next() == Some(file.oid.as_str()) && fields.next() == Some("blob"),
            "Не найден Git-объект исходников"
        );
        let size = fields
            .next()
            .context("Нет размера Git-объекта")?
            .parse::<u64>()?;
        ensure!(fields.next().is_none(), "Неверный размер Git-объекта");
        total = total
            .checked_add(size)
            .context("Исходники слишком большие")?;
        ensure!(
            total <= max_bytes,
            "Исходники превышают лимит {} МиБ",
            max_bytes / 1024 / 1024
        );
        ensure!(
            file.mode != "120000" || size <= 64 * 1024,
            "Слишком длинная ссылка исходников"
        );
        sizes.push(size);
    }
    ensure!(
        metadata.lines().count() == files.len(),
        "Git не сообщил все размеры исходников"
    );
    Ok(sizes)
}

#[cfg(test)]
mod tests;
