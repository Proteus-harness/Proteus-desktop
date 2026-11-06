use std::{fs, path::PathBuf, process::Command};

use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("build/verification/catalog-build-tests")
            .join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
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

    fn repository(&self, script: &str) -> (Source, Catalog) {
        let repo = self.0.join("repository");
        fs::create_dir_all(repo.join("packages/demo")).unwrap();
        fs::write(repo.join("packages/demo/README.md"), "# Build fixture\n").unwrap();
        fs::write(repo.join("build.py"), script).unwrap();
        fs::write(repo.join("source.txt"), "exact snapshot").unwrap();
        fs::write(
            repo.join(schema::INDEX),
            serde_json::json!({
                "format":1,"title":"Build fixture","categories":[{"id":"tools","title":"Tools"}],
                "entries":[{"id":"demo","kind":"client_extension","title":"Demo",
                "category":"tools","version":"1.0","path":"packages/demo",
                "source":{"path":".","build":["python3","build.py"],"artifact":"dist/demo.zip"}}]
            })
            .to_string(),
        )
        .unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["add", "."]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                "fixture",
            ],
        );
        let source = Source {
            url: repo.to_string_lossy().into_owned(),
            branch: None,
        };
        let catalog = git::refresh(&self.locations().cache, &source).unwrap();
        assert!(catalog.problems.is_empty(), "{:?}", catalog.problems);
        (source, catalog)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

const SUCCESS: &str = r#"import os, pathlib, zipfile
root = pathlib.Path('.')
assert os.environ['CARGO_BUILD_JOBS'] == '2'
assert pathlib.Path(os.environ['CARGO_TARGET_DIR']).is_absolute()
out = root / 'dist'
out.mkdir(exist_ok=True)
with zipfile.ZipFile(out / 'demo.zip', 'w') as z:
    z.writestr('source.txt', (root / 'source.txt').read_text())
    z.writestr('target.txt', os.environ['CARGO_TARGET_DIR'])
print('build complete')
"#;

#[test]
fn builds_frozen_snapshot_and_keeps_shared_target_without_installing() {
    let fixture = Fixture::new();
    let (source, catalog) = fixture.repository(SUCCESS);
    // A local working-tree edit must not contaminate the selected revision.
    fs::write(fixture.0.join("repository/source.txt"), "not committed").unwrap();
    let locations = fixture.locations();
    let built = prepare(&locations, &source, &catalog, &catalog.entries[0]).unwrap();
    assert_eq!(built.text("source.txt").unwrap(), "exact snapshot");
    let target = PathBuf::from(built.text("target.txt").unwrap());
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("cache-marker"), "keep").unwrap();
    let second = prepare(&locations, &source, &catalog, &catalog.entries[0]).unwrap();
    assert_eq!(second.text("target.txt").unwrap(), target.to_str().unwrap());
    assert_eq!(
        fs::read_to_string(target.join("cache-marker")).unwrap(),
        "keep"
    );
    assert!(!locations.extensions.exists());
    assert!(!locations.modules.exists());
    assert!(!fixture.0.join("repository/dist").exists());
}

#[test]
fn failed_build_does_not_touch_installation_or_accept_stale_archive() {
    let fixture = Fixture::new();
    let (source, catalog) =
        fixture.repository("print('specific build failure')\nraise SystemExit(7)\n");
    let locations = fixture.locations();
    let installed = locations.extensions.join("demo");
    fs::create_dir_all(&installed).unwrap();
    fs::write(installed.join("keep"), "old version").unwrap();
    let error = prepare(&locations, &source, &catalog, &catalog.entries[0])
        .err()
        .unwrap();
    let message = format!("{error:#}");
    assert!(message.contains("specific build failure"), "{message}");
    assert!(message.contains("7"), "{message}");
    assert_eq!(
        fs::read_to_string(installed.join("keep")).unwrap(),
        "old version"
    );
}

#[test]
fn successful_exit_requires_a_fresh_archive_from_this_build() {
    let fixture = Fixture::new();
    let (source, _) = fixture.repository("print('did not produce a package')\n");
    let repo = fixture.0.join("repository");
    fs::create_dir(repo.join("dist")).unwrap();
    // Even a valid archive committed in the source cannot be adopted after a
    // command which merely exits successfully without producing its output.
    let output = Command::new("python3").args(["-c", "import zipfile; z=zipfile.ZipFile('dist/demo.zip','w'); z.writestr('stale','old'); z.close()"])
        .current_dir(&repo).output().unwrap();
    assert!(output.status.success());
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "stale output",
        ],
    );
    let locations = fixture.locations();
    let catalog = git::refresh(&locations.cache, &source).unwrap();
    let error = prepare(&locations, &source, &catalog, &catalog.entries[0])
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("не создала указанный ZIP"));
}

#[test]
fn artifact_parent_symlink_cannot_remove_an_external_archive_before_a_failed_build() {
    let fixture = Fixture::new();
    let (source, _) = fixture.repository("print('build failed')\nraise SystemExit(7)\n");
    let outside = fixture.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let marker = outside.join("marker.zip");
    fs::write(&marker, "external archive must stay").unwrap();
    let repo = fixture.0.join("repository");
    std::os::unix::fs::symlink(&outside, repo.join("dist")).unwrap();
    let index_path = repo.join(schema::INDEX);
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    index["entries"][0]["source"]["artifact"] = "dist/marker.zip".into();
    fs::write(index_path, index.to_string()).unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "external artifact parent",
        ],
    );
    let locations = fixture.locations();
    let catalog = git::refresh(&locations.cache, &source).unwrap();
    let error = prepare(&locations, &source, &catalog, &catalog.entries[0])
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("Родитель ZIP сборки находится вне workspace"));
    assert_eq!(
        fs::read_to_string(marker).unwrap(),
        "external archive must stay"
    );
}

#[test]
fn artifact_parent_allows_new_directories_but_rejects_dangling_ancestors() {
    let fixture = Fixture::new();
    let workspace = fixture.0.join("workspace");
    fs::create_dir(&workspace).unwrap();
    assert!(ensure_artifact_parent(&workspace, &workspace.join("new/nested/package.zip")).is_ok());
    std::os::unix::fs::symlink(fixture.0.join("missing"), workspace.join("dangling")).unwrap();
    assert!(
        ensure_artifact_parent(&workspace, &workspace.join("dangling/nested/package.zip")).is_err()
    );
}

#[test]
fn selected_entry_metadata_comes_from_catalog_and_artifact_path_is_checked() {
    let fixture = Fixture::new();
    let (source, mut catalog) = fixture.repository(SUCCESS);
    let mut stale = catalog.entries[0].clone();
    stale.source.as_mut().unwrap().build = vec!["missing-stale-command".into()];
    assert!(prepare(&fixture.locations(), &source, &catalog, &stale).is_ok());
    catalog.entries[0].source.as_mut().unwrap().artifact = Some("../outside.zip".into());
    let error = prepare(&fixture.locations(), &source, &catalog, &stale)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("внутри каталога"));
}

#[test]
fn log_and_diagnostic_tail_are_bounded_while_all_output_is_consumed() {
    let input = vec![b'a'; LOG_BYTES * 2];
    let mut log = Vec::new();
    let tail = capture(input.as_slice(), &mut log, &AtomicBool::new(false)).unwrap();
    assert_eq!(log.len(), LOG_BYTES);
    assert_eq!(tail.len(), TAIL_BYTES);
    assert!(tail.bytes().all(|byte| byte == b'a'));
}

#[test]
fn finished_build_does_not_wait_forever_for_an_inherited_output_handle() {
    let (reader, mut inherited) = UnixStream::pair().unwrap();
    reader
        .set_read_timeout(Some(Duration::from_millis(20)))
        .unwrap();
    inherited.write_all(b"last diagnostic").unwrap();
    let start = Instant::now();
    let mut log = Vec::new();
    let tail = capture(reader, &mut log, &AtomicBool::new(true)).unwrap();
    assert_eq!(tail, "last diagnostic");
    assert!(start.elapsed() < Duration::from_secs(2));
    // The inherited writer deliberately stays open throughout capture.
    drop(inherited);
}
