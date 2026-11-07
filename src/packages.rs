//! Local package files shared by client extensions, git catalogs and ZIP
//! imports. No manifest semantics here: each consumer validates its format.

use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};

const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_FILES: usize = 4096;

struct File {
    content: Vec<u8>,
    mode: Option<u32>,
}
pub struct Contents(BTreeMap<PathBuf, File>);

impl Contents {
    pub fn zip(path: &Path) -> Result<Self> {
        ensure!(
            std::fs::metadata(path)?.len() <= 64 * 1024 * 1024,
            "ZIP превышает 64 МиБ"
        );
        Self::zip_bytes(&std::fs::read(path)?)
    }

    pub fn zip_bytes(bytes: &[u8]) -> Result<Self> {
        let mut archive =
            zip::ZipArchive::new(Cursor::new(bytes)).context("Не удалось прочитать ZIP")?;
        ensure!(
            archive.len() <= MAX_FILES,
            "В ZIP больше {MAX_FILES} записей"
        );
        let mut files = Self(BTreeMap::new());
        let mut total = 0u64;
        for ix in 0..archive.len() {
            let mut entry = archive.by_index(ix)?;
            let path = relative(entry.name())?;
            let mode = entry.unix_mode();
            ensure!(
                mode.is_none_or(|mode| mode & 0o170000 != 0o120000),
                "Ссылки в ZIP не поддерживаются"
            );
            if entry.is_dir() {
                continue;
            }
            let size = entry.size();
            total = total.checked_add(size).context("Слишком большой ZIP")?;
            ensure!(total <= MAX_BYTES, "Распакованный пакет превышает 128 МиБ");
            let mut content = Vec::new();
            entry.by_ref().take(size + 1).read_to_end(&mut content)?;
            ensure!(content.len() as u64 == size, "Неверный размер файла в ZIP");
            ensure!(
                files.0.insert(path, File { content, mode }).is_none(),
                "Повторный путь файла в ZIP"
            );
        }
        Ok(files)
    }

    pub fn folder(path: &Path) -> Result<Self> {
        let mut files = Self(BTreeMap::new());
        files.collect(path, Path::new(""), &mut 0)?;
        Ok(files)
    }

    fn collect(&mut self, root: &Path, parent: &Path, total: &mut u64) -> Result<()> {
        for entry in std::fs::read_dir(root.join(parent))? {
            let entry = entry?;
            ensure!(
                entry.file_name() != ".git",
                "Метаданные .git не входят в пакет"
            );
            let relative = parent.join(entry.file_name());
            let metadata = std::fs::symlink_metadata(entry.path())?;
            ensure!(
                !metadata.is_symlink(),
                "Ссылки в пакетах не поддерживаются: {}",
                relative.display()
            );
            if metadata.is_dir() {
                self.collect(root, &relative, total)?;
                continue;
            }
            ensure!(metadata.is_file(), "Ожидался обычный файл");
            *total += metadata.len();
            ensure!(
                *total <= MAX_BYTES && self.0.len() < MAX_FILES,
                "Слишком большой пакет"
            );
            use std::os::unix::fs::PermissionsExt;
            self.0.insert(
                relative,
                File {
                    content: std::fs::read(entry.path())?,
                    mode: Some(metadata.permissions().mode()),
                },
            );
        }
        Ok(())
    }

    pub fn text(&self, name: &str) -> Result<&str> {
        let file = self
            .0
            .get(Path::new(name))
            .with_context(|| format!("В корне пакета нужен {name}"))?;
        std::str::from_utf8(&file.content).with_context(|| format!("{name} не в UTF-8"))
    }

    /// Validated contents are written to a hidden sibling before publication.
    pub fn install(&self, root: &Path, id: &str, replace: bool) -> Result<PathBuf> {
        self.install_checked(root, id, replace, |_| Ok(()))
    }

    /// Check the complete staged package before touching an existing installation.
    /// Its directory has the final id so normal on-disk validators can be reused.
    pub fn install_checked(
        &self,
        root: &Path,
        id: &str,
        replace: bool,
        validate: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<PathBuf> {
        ensure!(
            !id.is_empty()
                && id.len() <= 128
                && id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'),
            "Неверное имя папки пакета"
        );
        let destination = root.join(id);
        ensure!(
            replace || !destination.exists(),
            "Папка уже существует: {}",
            destination.display()
        );
        std::fs::create_dir_all(root)?;
        let temporary = root.join(format!(".install-{}", uuid::Uuid::new_v4()));
        let result = (|| {
            std::fs::create_dir(&temporary)?;
            let staged = temporary.join(id);
            std::fs::create_dir(&staged)?;
            for (path, file) in &self.0 {
                let target = staged.join(path);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&target, &file.content)?;
                if let Some(mode) = file.mode {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        target,
                        std::fs::Permissions::from_mode(mode & 0o755 | 0o600),
                    )?;
                }
            }
            validate(&staged)?;
            publish(&staged, &destination, replace)?;
            Ok(destination)
        })();
        // On success publication moved the child; on failure discard only staging.
        let _ = std::fs::remove_dir_all(temporary);
        result
    }
}

fn relative(name: &str) -> Result<PathBuf> {
    let mut path = PathBuf::new();
    for part in Path::new(name).components() {
        match part {
            Component::Normal(name) if name != ".git" => path.push(name),
            Component::CurDir => {}
            _ => bail!("Путь выходит за пределы пакета: {name}"),
        }
    }
    ensure!(!path.as_os_str().is_empty(), "Пустой путь в ZIP");
    Ok(path)
}

static CHANGES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn try_change_guard() -> Result<Option<std::sync::MutexGuard<'static, ()>>> {
    match CHANGES.try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(std::sync::TryLockError::WouldBlock) => Ok(None),
        Err(std::sync::TryLockError::Poisoned(_)) => {
            anyhow::bail!("Предыдущая операция с пакетом прервана")
        }
    }
}

/// Package workers may run for different windows. Serialize publication and
/// removal so a failed replacement cannot restore over another operation.
pub(crate) fn change_guard() -> Result<std::sync::MutexGuard<'static, ()>> {
    CHANGES
        .lock()
        .map_err(|_| anyhow::anyhow!("Предыдущая операция с пакетом прервана"))
}

pub fn publish(temporary: &Path, destination: &Path, replace: bool) -> Result<()> {
    let _guard = change_guard()?;
    let backup = destination.with_file_name(format!(".previous-{}", uuid::Uuid::new_v4()));
    let exists = destination.exists();
    ensure!(
        !exists || replace,
        "Папка уже существует: {}",
        destination.display()
    );
    if exists {
        std::fs::rename(destination, &backup)?;
    }
    if let Err(error) = std::fs::rename(temporary, destination) {
        if exists {
            let _ = std::fs::rename(&backup, destination);
        }
        return Err(error.into());
    }
    if exists {
        let _ = std::fs::remove_dir_all(backup);
    }
    Ok(())
}
