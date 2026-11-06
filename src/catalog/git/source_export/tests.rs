use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::{Command, Stdio},
};

use super::*;

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    cache: PathBuf,
    source: Source,
}

impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("build/verification/catalog-source-export-tests")
            .join(uuid::Uuid::new_v4().to_string());
        let repository = root.join("repository");
        fs::create_dir_all(repository.join("components/demo")).unwrap();
        fs::create_dir_all(repository.join("components/runtime")).unwrap();
        fs::create_dir_all(repository.join("scripts")).unwrap();
        fs::write(repository.join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        fs::write(repository.join("components/demo/main.rs"), "old source\n").unwrap();
        fs::write(
            repository.join("components/runtime/lib.rs"),
            "workspace dependency\n",
        )
        .unwrap();
        fs::write(repository.join("LICENSE"), "license fixture\n").unwrap();
        fs::write(repository.join("scripts/build.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(
            repository.join("scripts/build.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("../../LICENSE", repository.join("components/demo/LICENSE")).unwrap();
        fs::write(
            repository.join(schema::INDEX),
            r#"{"format":1,"title":"Source test","categories":[],"entries":[]}"#,
        )
        .unwrap();
        let source = Source {
            url: repository.to_str().unwrap().to_owned(),
            branch: Some("main".into()),
        };
        let fixture = Self {
            cache: root.join("cache"),
            root,
            repository,
            source,
        };
        fixture.git(&["init", "-q", "-b", "main"]);
        fixture.git(&["add", "."]);
        fixture.commit();
        fixture
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.repository)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn commit(&self) {
        self.git(&[
            "-c",
            "user.name=Source Test",
            "-c",
            "user.email=source@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ]);
    }

    fn refresh(&self) -> String {
        crate::catalog::git::refresh(&self.cache, &self.source)
            .unwrap()
            .revision
            .unwrap()
    }

    fn assert_clean_staging(&self) {
        assert!(fs::read_dir(&self.root).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".source-")
        }));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn source_export_retains_selected_revision_workspace_modes_and_links_after_refresh() {
    let fixture = Fixture::new();
    let selected = fixture.refresh();
    assert_eq!(selected, fixture.git(&["rev-parse", "HEAD"]));
    fs::write(
        fixture.repository.join("components/demo/main.rs"),
        "new source\n",
    )
    .unwrap();
    fixture.git(&["add", "."]);
    fixture.commit();
    assert_ne!(fixture.refresh(), selected);
    let destination = fixture.root.join("downloaded");
    export_source(
        &fixture.cache,
        &fixture.source,
        &selected,
        Path::new("."),
        &destination,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("components/demo/main.rs")).unwrap(),
        "old source\n"
    );
    assert_eq!(
        fs::read_to_string(destination.join("components/runtime/lib.rs")).unwrap(),
        "workspace dependency\n"
    );
    assert!(destination.join("Cargo.toml").is_file());
    assert_eq!(
        fs::metadata(destination.join("scripts/build.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
    assert!(
        fs::symlink_metadata(destination.join("components/demo/LICENSE"))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(
        fs::read_link(destination.join("components/demo/LICENSE")).unwrap(),
        Path::new("../../LICENSE")
    );
    assert!(!destination.join(".git").exists());
    fixture.assert_clean_staging();
}

#[test]
fn source_export_never_replaces_existing_destinations() {
    let fixture = Fixture::new();
    let selected = fixture.refresh();
    let directory = fixture.root.join("existing-directory");
    fs::create_dir(&directory).unwrap();
    fs::write(directory.join("marker"), "keep").unwrap();
    let file = fixture.root.join("existing-file");
    fs::write(&file, "keep").unwrap();
    let link = fixture.root.join("existing-link");
    symlink("absent", &link).unwrap();
    for destination in [&directory, &file, &link] {
        assert!(
            export_source(
                &fixture.cache,
                &fixture.source,
                &selected,
                Path::new("."),
                destination
            )
            .is_err()
        );
    }
    assert_eq!(
        fs::read_to_string(directory.join("marker")).unwrap(),
        "keep"
    );
    assert_eq!(fs::read_to_string(&file).unwrap(), "keep");
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("absent"));
    fixture.assert_clean_staging();
}

#[test]
fn source_export_keeps_git_blob_bytes_without_checkout_filters() {
    let fixture = Fixture::new();
    fs::write(
        fixture.repository.join(".gitattributes"),
        "*.rs filter=source-export-test text eol=crlf\n",
    )
    .unwrap();
    fixture.git(&["add", "."]);
    fixture.commit();
    let selected = fixture.refresh();
    let private_repo = super::super::repo_path(&fixture.cache, &fixture.source);
    for (key, value) in [
        ("filter.source-export-test.smudge", "false"),
        ("filter.source-export-test.required", "true"),
    ] {
        assert!(
            Command::new("git")
                .arg("--git-dir")
                .arg(&private_repo)
                .args(["config", key, value])
                .status()
                .unwrap()
                .success()
        );
    }
    let destination = fixture.root.join("downloaded");
    export_source(
        &fixture.cache,
        &fixture.source,
        &selected,
        Path::new("."),
        &destination,
    )
    .unwrap();
    assert_eq!(
        fs::read(destination.join("components/demo/main.rs")).unwrap(),
        b"old source\n"
    );
    fixture.assert_clean_staging();
}

#[test]
fn source_export_checks_revision_and_root_and_can_export_a_subtree() {
    let fixture = Fixture::new();
    let selected = fixture.refresh();
    let destination = fixture.root.join("downloaded");
    assert!(
        export_source(
            &fixture.cache,
            &fixture.source,
            "main",
            Path::new("."),
            &destination
        )
        .is_err()
    );
    for root in ["/", "../repository", ".git", "missing", "LICENSE"] {
        assert!(
            export_source(
                &fixture.cache,
                &fixture.source,
                &selected,
                Path::new(root),
                &destination
            )
            .is_err(),
            "{root}"
        );
        assert!(!destination.exists());
    }
    export_source(
        &fixture.cache,
        &fixture.source,
        &selected,
        Path::new("components/runtime"),
        &destination,
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(destination.join("lib.rs")).unwrap(),
        "workspace dependency\n"
    );
    assert!(!destination.join("components").exists());
    fixture.assert_clean_staging();
}

#[test]
fn source_export_enforces_file_and_byte_budgets_before_publication() {
    let fixture = Fixture::new();
    let selected = fixture.refresh();
    let destination = fixture.root.join("downloaded");
    for (files, bytes) in [(1, MAX_BYTES), (MAX_FILES, 1)] {
        assert!(
            export_bounded(
                &fixture.cache,
                &fixture.source,
                &selected,
                Path::new("."),
                &destination,
                files,
                bytes
            )
            .is_err()
        );
        assert!(!destination.exists());
        fixture.assert_clean_staging();
    }
}

#[test]
fn source_export_removes_partial_stage_if_materialization_fails() {
    let fixture = Fixture::new();
    // Git can store a link target longer than the filesystem permits. This
    // fails during materialization after earlier regular files were written.
    let mut child = Command::new("git")
        .arg("-C")
        .arg(&fixture.repository)
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&vec![b'x'; 5000])
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let hash = String::from_utf8(output.stdout).unwrap().trim().to_owned();
    fixture.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        "120000",
        &hash,
        "invalid-link",
    ]);
    fixture.commit();
    let selected = fixture.refresh();
    let destination = fixture.root.join("downloaded");
    assert!(
        export_source(
            &fixture.cache,
            &fixture.source,
            &selected,
            Path::new("."),
            &destination
        )
        .is_err()
    );
    assert!(!destination.exists());
    fixture.assert_clean_staging();
}
