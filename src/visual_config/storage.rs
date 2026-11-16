//! Bounded pure-visual JSON/TOML files, atomically replaced beside their source.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::VisualConfig;

const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
enum Format {
    Json,
    Toml,
}

fn format(path: &Path) -> Result<Format> {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("json") => Ok(Format::Json),
        Some("toml") => Ok(Format::Toml),
        _ => bail!("visual configuration must be a JSON or TOML file"),
    }
}

/// Resolve a selected document and reject a selected agent file, including
/// symlinks and hard links. Config parsing is separate and never falls back.
pub fn select_path(path: &Path, agent: Option<&Path>) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("visual configuration is unavailable: {}", path.display()))?;
    if !path.is_file() {
        bail!("visual configuration is not a file: {}", path.display());
    }
    format(&path)?;
    protect_agent(&path, agent)?;
    Ok(path)
}

pub fn load(path: &Path, agent: Option<&Path>) -> Result<VisualConfig> {
    let path = select_path(path, agent)?;
    let mut bytes = Vec::new();
    File::open(&path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        bail!("visual configuration exceeds {MAX_BYTES} bytes");
    }
    let text = std::str::from_utf8(&bytes).context("visual configuration is not UTF-8")?;
    let value: VisualConfig = match format(&path)? {
        Format::Json => serde_json::from_str(text).context("invalid visual JSON configuration")?,
        Format::Toml => toml::from_str(text).context("invalid visual TOML configuration")?,
    };
    value.validate()?;
    Ok(value)
}

/// Save without editing an agent or app metadata file. The destination's
/// extension chooses the format; invalid values fail before any write.
pub fn save(path: &Path, value: &VisualConfig, agent: Option<&Path>) -> Result<PathBuf> {
    value.validate()?;
    let path = destination(path)?;
    let selected_format = format(&path)?;
    protect_agent(&path, agent)?;
    if path.exists() {
        load(&path, agent).context(
            "refusing to replace a file that is not a valid Desktop visual configuration",
        )?;
    }
    let text = match selected_format {
        Format::Json => serde_json::to_string_pretty(value)? + "\n",
        Format::Toml => toml::to_string_pretty(value)?,
    };
    if text.len() > MAX_BYTES {
        bail!("visual configuration exceeds {MAX_BYTES} bytes");
    }
    let parent = path
        .parent()
        .context("visual configuration has no parent")?;
    let draft = parent.join(format!(".visual-config-{}.pending", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&draft)?;
        if let Ok(metadata) = fs::metadata(&path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        protect_agent(&path, agent)?;
        fs::rename(&draft, &path)?;
        Ok(path.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&draft);
    }
    result
}

fn destination(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        let path = path.canonicalize()?;
        if !path.is_file() {
            bail!("visual configuration destination is not a file");
        }
        return Ok(path);
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let filename = absolute
        .file_name()
        .context("visual configuration needs a filename")?;
    let parent = absolute
        .parent()
        .context("visual configuration has no parent")?
        .canonicalize()?;
    Ok(parent.join(filename))
}

fn protect_agent(visual: &Path, agent: Option<&Path>) -> Result<()> {
    let Some(agent) = agent else {
        return Ok(());
    };
    let agent = agent
        .canonicalize()
        .with_context(|| format!("agent configuration is unavailable: {}", agent.display()))?;
    if visual == agent || same_file(visual, &agent)? {
        bail!("a Desktop visual configuration cannot replace the agent configuration");
    }
    Ok(())
}

fn same_file(first: &Path, second: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt as _;
    let first = match fs::metadata(first) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let second = fs::metadata(second)?;
    Ok(first.dev() == second.dev() && first.ino() == second.ino())
}
