use super::*;

fn source_repo(fixture: &Fixture) -> PathBuf {
    let repo = fixture.0.join("source-repo");
    fs::create_dir_all(repo.join("components/demo")).unwrap();
    fs::create_dir_all(repo.join("tool/demo")).unwrap();
    fs::write(repo.join("Cargo.toml"), "workspace fixture").unwrap();
    fs::write(repo.join("tool/demo/lib.rs"), "selected dependency\n").unwrap();
    fs::write(repo.join("README.md"), "# Workspace\n").unwrap();
    fs::write(repo.join("components/demo/Cargo.toml"), "../../tool/demo\n").unwrap();
    let index = serde_json::json!({
        "format": 1, "title": "Source modules", "categories": [{"id":"tools","title":"Tools"}],
        "entries": [{"id":"demo", "kind":"process_module", "title":"Demo", "category":"tools",
        "version":"1.0.0", "path":"components/demo", "source": {"path":".",
        "build":["sh","-c","touch SHOULD_NOT_RUN"], "requirements":["Rust"]}}],
    });
    fs::write(repo.join(schema::INDEX), index.to_string()).unwrap();
    git_at(&repo, &["init", "-q", "-b", "main"]);
    commit(&repo);
    repo
}

#[test]
fn official_sources_are_added_once_without_replacing_user_choices() {
    let custom = source(Path::new("custom"));
    let mut preferences: Preferences = serde_json::from_value(serde_json::json!({
        "sources":[custom, {"url":"https://github.com/Proteus-harness/Proteus-modules/","branch":null}],
        "installed":[],
    })).unwrap();
    preferences.ensure_official_sources();
    assert_eq!(preferences.sources.len(), 3);
    assert!(preferences.sources.contains(&custom));
    assert!(open::OFFICIAL[0].matches(&preferences.sources[1]));
    preferences
        .sources
        .retain(|item| item.url != open::OFFICIAL[1].url);
    let mut reopened: Preferences =
        serde_json::from_slice(&serde_json::to_vec(&preferences).unwrap()).unwrap();
    reopened.ensure_official_sources();
    assert_eq!(
        reopened.sources, preferences.sources,
        "removed official source stays removed"
    );
}

#[test]
fn source_download_keeps_workspace_and_instructions_without_install_or_execution() {
    let fixture = Fixture::new();
    let repo = source_repo(&fixture);
    let repository = source(&repo);
    let locations = fixture.locations();
    let first = git::refresh(&locations.cache, &repository).unwrap();
    assert!(first.problems.is_empty());
    let entry = &first.entries[0];
    assert_eq!(entry.kind, Kind::ProcessModule);
    assert_eq!(
        entry.source.as_ref().unwrap().command(),
        "sh -c 'touch SHOULD_NOT_RUN'"
    );
    fs::write(repo.join("tool/demo/lib.rs"), "new revision\n").unwrap();
    commit(&repo);
    git::refresh(&locations.cache, &repository).unwrap();
    let downloaded = download::source(&locations, &repository, &first, entry, &fixture.0).unwrap();
    assert_eq!(
        fs::read_to_string(downloaded.join("source/tool/demo/lib.rs")).unwrap(),
        "selected dependency\n"
    );
    assert!(downloaded.join("source/Cargo.toml").is_file());
    assert!(
        downloaded
            .join("source/components/demo/Cargo.toml")
            .is_file()
    );
    let instructions = fs::read_to_string(downloaded.join("BUILD.md")).unwrap();
    assert!(instructions.contains("cd source\nsh -c 'touch SHOULD_NOT_RUN'"));
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(downloaded.join("source.json")).unwrap()).unwrap();
    assert_eq!(metadata["revision"].as_str(), first.revision.as_deref());
    assert!(!repo.join("SHOULD_NOT_RUN").exists());
    assert!(!downloaded.join("source/SHOULD_NOT_RUN").exists());
    assert!(!locations.extensions.exists());
    assert!(!locations.plugins.exists());
    fs::write(downloaded.join("keep"), "untouched").unwrap();
    assert!(download::source(&locations, &repository, &first, entry, &fixture.0).is_err());
    assert_eq!(
        fs::read_to_string(downloaded.join("keep")).unwrap(),
        "untouched"
    );
    assert!(!fs::read_dir(&fixture.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".proteus-source-")
    }));
}

#[test]
fn source_schema_rejects_missing_build_and_workspace_outside_entry() {
    let fixture = Fixture::new();
    let repo = source_repo(&fixture);
    let original: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.join(schema::INDEX)).unwrap()).unwrap();
    for bad in [
        serde_json::Value::Null,
        serde_json::json!({"path":".","build":[]}),
        serde_json::json!({"path":"tool","build":["cargo","build"]}),
        serde_json::json!({"path":"../","build":["cargo","build"]}),
    ] {
        let mut index = original.clone();
        index["entries"][0]["source"] = bad;
        fs::write(repo.join(schema::INDEX), index.to_string()).unwrap();
        let catalog = schema::read(&repo, "bad", &Default::default()).unwrap();
        assert!(catalog.entries.is_empty());
        assert_eq!(catalog.problems.len(), 1);
    }
}

#[test]
fn source_download_preserves_an_existing_dangling_symlink() {
    let fixture = Fixture::new();
    let repo = source_repo(&fixture);
    let repository = source(&repo);
    let locations = fixture.locations();
    let catalog = git::refresh(&locations.cache, &repository).unwrap();
    let name = format!("demo-{}", &catalog.revision.as_deref().unwrap()[..12]);
    let destination = fixture.0.join(name);
    let missing = fixture.0.join("missing-target");
    std::os::unix::fs::symlink(&missing, &destination).unwrap();
    assert!(
        download::source(
            &locations,
            &repository,
            &catalog,
            &catalog.entries[0],
            &fixture.0
        )
        .is_err()
    );
    assert_eq!(fs::read_link(destination).unwrap(), missing);
}

#[test]
fn source_download_uses_entry_and_revision_from_one_snapshot_after_refresh() {
    let fixture = Fixture::new();
    let repo = source_repo(&fixture);
    let repository = source(&repo);
    let locations = fixture.locations();
    let old = git::refresh(&locations.cache, &repository).unwrap();
    let mut index: serde_json::Value =
        serde_json::from_slice(&fs::read(repo.join(schema::INDEX)).unwrap()).unwrap();
    index["entries"][0]["version"] = "2.0.0".into();
    index["entries"][0]["source"]["build"] = serde_json::json!(["cargo", "build"]);
    fs::write(repo.join(schema::INDEX), index.to_string()).unwrap();
    commit(&repo);
    let current = git::refresh(&locations.cache, &repository).unwrap();
    let path = download::source(
        &locations,
        &repository,
        &current,
        &old.entries[0],
        &fixture.0,
    )
    .unwrap();
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(path.join("source.json")).unwrap()).unwrap();
    assert_eq!(metadata["version"], "2.0.0");
    assert_eq!(metadata["revision"].as_str(), current.revision.as_deref());
    assert_eq!(metadata["build"], serde_json::json!(["cargo", "build"]));
    assert!(
        fs::read_to_string(path.join("BUILD.md"))
            .unwrap()
            .contains("cd source\ncargo build")
    );
}
