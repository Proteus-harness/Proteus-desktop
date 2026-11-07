//! Installing extensions from ZIP archives and removing them.
//!
//! The archive holds the extension folder contents with `extension.json` at
//! its root. It is unpacked into a hidden folder first and renamed into
//! `<root>/<id>` only after the normal on-disk manifest checks pass.
//! An incomplete update never replaces the previous installation.

use std::path::Path;

use anyhow::{Result, ensure};

use crate::packages::Contents;

use super::manifest::{MANIFEST, Manifest, parse};

/// Unpacks the archive at `archive` into `<root>/<id>`. `taken` are ids the
/// new extension must not reuse besides the folders already in `root`.
pub fn install(root: &Path, archive: &Path, taken: &[&str]) -> Result<Manifest> {
    install_contents(root, Contents::zip(archive)?, taken)
}

#[cfg(test)]
fn install_bytes(root: &Path, bytes: &[u8], taken: &[&str]) -> Result<Manifest> {
    install_contents(root, Contents::zip_bytes(bytes)?, taken)
}

fn install_contents(root: &Path, files: Contents, taken: &[&str]) -> Result<Manifest> {
    let manifest = parse(files.text(MANIFEST)?)?;
    ensure!(
        !taken.contains(&manifest.id.as_str()),
        "Расширение `{}` уже установлено или его имя занято",
        manifest.id
    );
    let existing = root.join(&manifest.id);
    let replace = existing.exists();
    if replace {
        let previous = parse(&std::fs::read_to_string(existing.join(MANIFEST))?)?;
        ensure!(
            previous.id == manifest.id,
            "Существующая папка не принадлежит этому расширению"
        );
    }
    files.install_checked(root, &manifest.id, replace, |dir| {
        super::manifest::load(dir).map(|_| ())
    })?;
    Ok(manifest)
}

/// Deletes the folder of an installed extension.
pub fn remove(root: &Path, id: &str) -> Result<()> {
    remove_with(root, id, || ())
}

/// Capture publication state under the same lock as deletion.
pub(crate) fn remove_with<T>(root: &Path, id: &str, after: impl FnOnce() -> T) -> Result<T> {
    let _guard = crate::packages::change_guard()?;
    let dir = root.join(id);
    ensure!(
        dir.parent() == Some(root) && !id.is_empty() && !id.starts_with('.'),
        "Неверное имя расширения `{id}`"
    );
    if dir.try_exists()? {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(after())
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Cursor, Write as _},
        path::PathBuf,
    };

    use zip::write::SimpleFileOptions;

    use super::*;

    const MANIFEST_TEXT: &str = r#"{"protocol": 4, "id": "todo", "title": "Задачи", "command": ["./run"],
        "surfaces": {"panel": {"title": "Задачи"}}}"#;

    fn archive(files: &[(&str, &str, u32)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, text, mode) in files {
            let options = SimpleFileOptions::default().unix_permissions(*mode);
            writer.start_file(*name, options).unwrap();
            writer.write_all(text.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn temp_root() -> PathBuf {
        std::env::temp_dir().join(format!("proteus-zip-{}", uuid::Uuid::new_v4()))
    }

    fn skill_manifest(title: &str) -> String {
        let mut manifest: serde_json::Value = serde_json::from_str(MANIFEST_TEXT).unwrap();
        manifest["title"] = serde_json::json!(title);
        // A preflight must not try to spawn or resolve the extension's command.
        manifest["command"] = serde_json::json!(["proteus-preflight-must-not-run"]);
        manifest["required_features"] = serde_json::json!(["manifest.agent_instructions.v1"]);
        manifest["agent_instructions"] =
            serde_json::json!([{"skill": "guide", "path": "skills/guide"}]);
        manifest.to_string()
    }

    #[test]
    fn incomplete_skill_links_leave_no_new_installation() {
        let root = temp_root();
        let text = skill_manifest("New");
        assert!(parse(&text).is_ok());
        for extra in [
            vec![],
            vec![("skills/guide/README.md", "No skill file", 0o644)],
            vec![("skills/guide", "Not a directory", 0o644)],
            vec![(
                "skills/guide/SKILL.md/child",
                "Not a regular skill file",
                0o644,
            )],
        ] {
            let mut files = vec![("extension.json", text.as_str(), 0o644)];
            files.extend(extra);
            assert!(install_bytes(&root, &archive(&files), &[]).is_err());
            assert!(!root.join("todo").exists());
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incomplete_skill_update_keeps_the_previous_package_and_cleans_staging() {
        let root = temp_root();
        let original = skill_manifest("Original");
        let good = archive(&[
            ("extension.json", original.as_str(), 0o644),
            ("skills/guide/SKILL.md", "# Original instructions", 0o644),
            ("old-asset.txt", "Keep this until a valid update", 0o644),
        ]);
        install_bytes(&root, &good, &[]).unwrap();
        let changed = skill_manifest("Incomplete");
        for extra in [
            vec![],
            vec![("skills/guide/README.md", "No skill file", 0o644)],
            vec![("skills/guide", "Not a directory", 0o644)],
            vec![(
                "skills/guide/SKILL.md/child",
                "Not a regular skill file",
                0o644,
            )],
        ] {
            let mut files = vec![("extension.json", changed.as_str(), 0o644)];
            files.extend(extra);
            assert!(install_bytes(&root, &archive(&files), &[]).is_err());
            assert_eq!(
                std::fs::read_to_string(root.join("todo/extension.json")).unwrap(),
                original
            );
            assert_eq!(
                std::fs::read_to_string(root.join("todo/skills/guide/SKILL.md")).unwrap(),
                "# Original instructions"
            );
            assert_eq!(
                std::fs::read_to_string(root.join("todo/old-asset.txt")).unwrap(),
                "Keep this until a valid update"
            );
            assert_eq!(
                super::super::manifest::load(&root.join("todo"))
                    .unwrap()
                    .title,
                "Original"
            );
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn complete_skill_update_passes_discovery_without_running_the_command() {
        let root = temp_root();
        let original = skill_manifest("Original");
        install_bytes(
            &root,
            &archive(&[
                ("extension.json", original.as_str(), 0o644),
                ("skills/guide/SKILL.md", "# Original", 0o644),
                ("old-asset.txt", "Obsolete", 0o644),
            ]),
            &[],
        )
        .unwrap();
        let changed = skill_manifest("Updated");
        let installed = install_bytes(
            &root,
            &archive(&[
                ("extension.json", changed.as_str(), 0o644),
                ("skills/guide/SKILL.md", "# Updated", 0o644),
            ]),
            &[],
        )
        .unwrap();
        let (found, problems) = super::super::manifest::discover(&root);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest, installed);
        assert_eq!(installed.title, "Updated");
        assert_eq!(
            std::fs::read_to_string(root.join("todo/skills/guide/SKILL.md")).unwrap(),
            "# Updated"
        );
        assert!(!root.join("todo/old-asset.txt").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn installs_and_updates_keeping_executables_and_removes() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp_root();
        let bytes = archive(&[
            ("extension.json", MANIFEST_TEXT, 0o644),
            ("run", "#!/bin/sh\n", 0o755),
            ("lib/data.txt", "x", 0o644),
        ]);

        let manifest = install_bytes(&root, &bytes, &[]).unwrap();
        assert_eq!(manifest.id, "todo");
        let mode = std::fs::metadata(root.join("todo/run"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
        assert!(root.join("todo/lib/data.txt").is_file());
        let changed = archive(&[
            (
                "extension.json",
                &MANIFEST_TEXT.replace("Задачи", "Новые задачи"),
                0o644,
            ),
            ("run", "#!/bin/sh\necho updated\n", 0o755),
        ]);
        assert_eq!(
            install_bytes(&root, &changed, &[]).unwrap().title,
            "Новые задачи"
        );
        assert!(!root.join("todo/lib/data.txt").exists());
        assert!(
            std::fs::read_to_string(root.join("todo/run"))
                .unwrap()
                .contains("updated")
        );
        let bad = archive(&[("extension.json", "{}", 0o644)]);
        assert!(install_bytes(&root, &bad, &[]).is_err());
        assert!(
            std::fs::read_to_string(root.join("todo/run"))
                .unwrap()
                .contains("updated")
        );

        remove(&root, "todo").unwrap();
        let left: Vec<_> = std::fs::read_dir(&root).unwrap().collect();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(left.is_empty());
    }

    #[test]
    fn rejects_escaping_paths_taken_ids_and_missing_manifest() {
        let root = temp_root();
        let escaping = archive(&[
            ("extension.json", MANIFEST_TEXT, 0o644),
            ("../evil", "x", 0o644),
        ]);
        assert!(install_bytes(&root, &escaping, &[]).is_err());
        let fine = archive(&[("extension.json", MANIFEST_TEXT, 0o644)]);
        assert!(install_bytes(&root, &fine, &["todo"]).is_err());
        let nested = archive(&[("todo/extension.json", MANIFEST_TEXT, 0o644)]);
        assert!(install_bytes(&root, &nested, &[]).is_err());
        assert!(!root.exists() || std::fs::read_dir(&root).unwrap().next().is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
