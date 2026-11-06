//! Immutable module packages. Core owns their registration and activation.

use std::{collections::BTreeMap, io::Read, path::Path};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::super::{
    Locations, Target,
    schema::{self, Entry},
};
use crate::packages::Contents;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: String,
    version: String,
    command: Vec<String>,
    exports: BTreeMap<String, Vec<String>>,
    sha256: String,
}

pub(super) fn prepare(locations: &Locations, entry: &Entry, files: &Contents) -> Result<Target> {
    let manifest: Manifest =
        serde_json::from_str(files.text(&format!("share/modules/{}/module.json", entry.id))?)
            .context("Неверный module.json")?;
    ensure!(
        manifest.id == entry.id && manifest.version == entry.version,
        "id и версия module.json должны совпадать с каталогом"
    );
    let executable = manifest.command.first().context("У модуля нет команды")?;
    schema::relative(Path::new(executable))?;
    ensure!(
        manifest.command.iter().all(|arg| !arg.contains('\0')),
        "Неверные аргументы модуля"
    );
    ensure!(
        !manifest.exports.is_empty() && manifest.exports.values().all(|ids| !ids.is_empty()),
        "У модуля нет экспортов"
    );
    ensure!(
        manifest.sha256.len() == 64 && manifest.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "Неверная контрольная сумма модуля"
    );
    let id = format!("{}-{}", entry.id, uuid::Uuid::new_v4());
    let path = files.install_checked(&locations.modules, &id, false, |dir| {
        let executable = dir.join(executable);
        let metadata = std::fs::metadata(&executable)?;
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.is_file() && metadata.permissions().mode() & 0o111 != 0,
            "Команда модуля должна быть исполняемым файлом"
        );
        let mut file = std::fs::File::open(executable)?;
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
            "Контрольная сумма исполняемого файла не совпала с module.json"
        );
        Ok(())
    })?;
    let mut command = manifest.command;
    command[0] = path
        .join(&command[0])
        .to_str()
        .context("Путь модуля не в UTF-8")?
        .to_owned();
    Ok(Target::Component {
        path,
        command,
        exports: manifest.exports,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::schema::Kind;
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let root = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("build/verification")
                .join(format!("component-install-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            Self(root)
        }
        fn locations(&self) -> Locations {
            Locations {
                cache: self.0.join("cache"),
                extensions: self.0.join("extensions"),
                plugins: self.0.join("plugins"),
                skills: self.0.join("skills"),
                modules: self.0.join("modules"),
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn package(fixture: &Fixture, digest: &str) -> (Entry, Contents) {
        let root = fixture.0.join("package");
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("share/modules/demo")).unwrap();
        std::fs::write(root.join("bin/demo"), b"synthetic module").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            root.join("bin/demo"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(
            root.join("share/modules/demo/module.json"),
            serde_json::json!({
                "id":"demo", "version":"1", "command":["bin/demo"],
                "exports":{"tool":["demo_tool"]}, "sha256":digest,
            })
            .to_string(),
        )
        .unwrap();
        let entry = Entry {
            id: "demo".into(),
            kind: Kind::ProcessModule,
            title: "Demo".into(),
            description: String::new(),
            category: "tools".into(),
            version: "1".into(),
            path: ".".into(),
            source: None,
        };
        (entry, Contents::folder(&root).unwrap())
    }

    #[test]
    fn validated_packages_are_immutable_and_leave_old_binary_intact() {
        let fixture = Fixture::new();
        let (entry, files) = package(
            &fixture,
            &format!("{:x}", Sha256::digest(b"synthetic module")),
        );
        let first = prepare(&fixture.locations(), &entry, &files).unwrap();
        let second = prepare(&fixture.locations(), &entry, &files).unwrap();
        let Target::Component {
            path: old, command, ..
        } = first
        else {
            panic!()
        };
        let Target::Component { path: new, .. } = second else {
            panic!()
        };
        assert_ne!(old, new);
        assert!(Path::new(&command[0]).is_absolute());
        assert_eq!(
            std::fs::read(old.join("bin/demo")).unwrap(),
            b"synthetic module"
        );
    }

    #[test]
    fn invalid_checksum_never_publishes_module_package() {
        let fixture = Fixture::new();
        let (entry, files) = package(&fixture, &"0".repeat(64));
        assert!(prepare(&fixture.locations(), &entry, &files).is_err());
        assert_eq!(
            std::fs::read_dir(fixture.locations().modules)
                .unwrap()
                .count(),
            0
        );
    }
}
