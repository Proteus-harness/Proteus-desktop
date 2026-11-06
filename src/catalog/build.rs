//! A selected catalog build runs off the UI thread from its exact Git snapshot.
//! Only a successfully built, validated ZIP is handed to the installer. One
//! workspace/log per source is retained; Cargo's target cache is reused.

use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt as _},
    },
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, ensure};

use super::{
    Locations, Source, git,
    schema::{self, Catalog, Entry},
};
use crate::packages::Contents;

const LOG_BYTES: usize = 1024 * 1024;
const TAIL_BYTES: usize = 16 * 1024;
const TIMEOUT: Duration = Duration::from_secs(60 * 60);
static LOCKS: LazyLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = LazyLock::new(Mutex::default);

/// Build just this entry. Nothing is installed, replaced or connected here.
pub fn prepare(
    locations: &Locations,
    repository: &Source,
    catalog: &Catalog,
    selected: &Entry,
) -> Result<Contents> {
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.id == selected.id && entry.kind == selected.kind)
        .context("Запись изменилась; обновите выбор в каталоге")?;
    let package = entry.source.as_ref().context("У записи нет исходников")?;
    let artifact = package
        .artifact
        .as_ref()
        .context("В каталоге не указан ZIP для установки")?;
    schema::relative(artifact)?;
    ensure!(
        artifact
            .extension()
            .is_some_and(|extension| extension == "zip"),
        "Результат сборки должен быть ZIP-пакетом"
    );
    let revision = catalog
        .revision
        .as_deref()
        .context("Обновите Git-источник перед установкой")?;
    ensure!(
        matches!(revision.len(), 40 | 64) && revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Неверная ревизия источника"
    );
    let lock = LOCKS
        .lock()
        .map_err(|_| anyhow::anyhow!("Сборка каталога прервана"))?
        .entry(repository.key())
        .or_default()
        .clone();
    let _guard = lock
        .lock()
        .map_err(|_| anyhow::anyhow!("Предыдущая сборка прервана"))?;
    let root = locations.cache.join("builds").join(repository.key());
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    let workspaces = root.join("workspaces");
    // Export afresh even for the same revision: a preceding build may have
    // generated files. Keep the expensive target cache outside the workspace.
    if workspaces.exists() {
        fs::remove_dir_all(&workspaces)?;
    }
    fs::create_dir(&workspaces)?;
    let workspace = workspaces.join(revision);
    git::export_source(
        &locations.cache,
        repository,
        revision,
        &package.path,
        &workspace,
    )?;
    let output = workspace.join(artifact);
    ensure_artifact_parent(&workspace, &output)?;
    // A checked-in/stale archive must never count as a successful fresh build.
    match fs::symlink_metadata(&output) {
        Ok(metadata) if metadata.is_file() || metadata.is_symlink() => fs::remove_file(&output)?,
        Ok(_) => anyhow::bail!("На месте ZIP находится папка: {}", output.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    run(
        &package.build,
        &workspace,
        &root.join("target"),
        &root.join("build.log"),
    )?;
    let output = output
        .canonicalize()
        .context("Сборка не создала указанный ZIP-пакет")?;
    ensure!(
        output.starts_with(&workspace) && output.is_file(),
        "ZIP сборки находится вне workspace"
    );
    Contents::zip(&output).context("Сборка создала некорректный ZIP-пакет")
}

/// Validate before even removing an old output: Git sources may contain a
/// symlink in the artifact's parent path. Missing directories are fine, but
/// their nearest existing ancestor must resolve inside the exported workspace.
fn ensure_artifact_parent(workspace: &Path, output: &Path) -> Result<()> {
    let workspace = workspace.canonicalize()?;
    let mut parent = output.parent().context("У ZIP нет родительской папки")?;
    loop {
        match fs::symlink_metadata(parent) {
            Ok(_) => {
                let resolved = parent
                    .canonicalize()
                    .context("Не удалось разрешить родительскую папку ZIP")?;
                ensure!(
                    resolved.starts_with(&workspace) && resolved.is_dir(),
                    "Родитель ZIP сборки находится вне workspace"
                );
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                parent = parent
                    .parent()
                    .context("У ZIP нет существующего родителя")?;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn run(argv: &[String], workspace: &Path, target: &Path, log: &Path) -> Result<()> {
    let program = argv.first().context("Не указана команда сборки")?;
    let (reader, writer) = UnixStream::pair()?;
    reader.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut command = Command::new(program);
    command
        .args(&argv[1..])
        .current_dir(workspace)
        .env("CARGO_BUILD_JOBS", "2")
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_TERM_COLOR", "never")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(writer.try_clone()?)))
        .stderr(Stdio::from(OwnedFd::from(writer)))
        .process_group(0);
    // SAFETY: the child executes only async-signal-safe prctl before exec.
    unsafe {
        command.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let log_file = File::create(log)?;
    let mut child = command
        .spawn()
        .with_context(|| format!("Не удалось запустить {program}"))?;
    // Command owns the parent's pipe handles even after spawn.
    drop(command);
    let finished = Arc::new(AtomicBool::new(false));
    let reader_finished = finished.clone();
    let output = std::thread::spawn(move || capture(reader, log_file, &reader_finished));
    let start = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() < TIMEOUT => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => break Err(anyhow::anyhow!("Сборка превысила 60 минут")),
            Err(error) => break Err(error.into()),
        }
    };
    // The command must not leave helpers holding the log stream open after
    // completion or a timeout. The process group belongs to this build only.
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.wait();
    finished.store(true, Ordering::Release);
    let tail = output
        .join()
        .map_err(|_| anyhow::anyhow!("Не удалось прочитать журнал сборки"))??;
    let status = result.with_context(|| format!("Журнал: {}\n{tail}", log.display()))?;
    ensure!(
        status.success(),
        "Сборка завершилась с {status}. Журнал: {}\n{tail}",
        log.display()
    );
    Ok(())
}

/// Drain all output so verbose builds cannot block on a full pipe. Bound
/// both retained memory and the on-disk log, keeping the diagnostic tail.
fn capture(mut input: impl Read, mut log: impl Write, finished: &AtomicBool) -> Result<String> {
    let mut tail = VecDeque::with_capacity(TAIL_BYTES);
    let mut written = 0;
    let mut buffer = [0; 8192];
    let mut drain_until = None;
    loop {
        if finished.load(Ordering::Acquire) {
            let deadline =
                drain_until.get_or_insert_with(|| Instant::now() + Duration::from_millis(200));
            if Instant::now() >= *deadline {
                break;
            }
        }
        let count = match input.read(&mut buffer) {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        };
        if count == 0 {
            break;
        }
        let keep = count.min(LOG_BYTES.saturating_sub(written));
        log.write_all(&buffer[..keep])?;
        written += keep;
        for byte in &buffer[..count] {
            if tail.len() == TAIL_BYTES {
                tail.pop_front();
            }
            tail.push_back(*byte);
        }
    }
    log.flush()?;
    Ok(String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned())
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
