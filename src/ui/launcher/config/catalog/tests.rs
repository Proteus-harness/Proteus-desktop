use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("build/config-catalog-tests")
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn file(&self, path: &str) -> PathBuf {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Unknown documents stay visible; discovery never validates an agent
        // config or queries a provider while projecting profile labels.
        std::fs::write(&path, "not a configuration document").unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[::core::prelude::v1::test]
fn catalog_lists_profile_locations_without_reading_fragments_secrets_or_backups() {
    let fixture = Fixture::new();
    fixture.file("custom.toml");
    fixture.file("configs/config.toml");
    fixture.file("configs/codex.config.toml");
    let selected = fixture.file("opencode-go/configs/codex.config.toml");
    fixture.file("configs/fragments/partial.toml");
    fixture.file("configs/prompts/prompt.json");
    fixture.file("configs/proteus.provider.example.toml");
    fixture.file("secrets/auth.json");
    fixture.file("secrets/configs/secret.toml");
    fixture.file("sessions/configs/snapshot.json");
    fixture.file("configs.bak/copy.toml");
    fixture.file("configs-backup-old/configs/copy.toml");
    fixture.file("config-history/configs/copy.toml");
    fixture.file("model-selections/configs/copy.json");
    fixture.file(".proteus/configs/snapshot.toml");
    fixture.file("opencode-go/runtime/configs/worker.json");
    fixture.file("opencode-go/irrelevant.json");
    let catalog = discover(Some(fixture.0.clone()), Some(selected));
    assert!(catalog.problem.is_none());
    let names: Vec<_> = catalog
        .files
        .iter()
        .map(|file| file.relative.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "configs/codex.config.toml",
            "opencode-go/configs/codex.config.toml",
            "configs/config.toml",
            "custom.toml"
        ]
    );
    assert_eq!(catalog.files.iter().filter(|file| file.current).count(), 1);
    assert!(catalog.files[1].current);
    assert!(catalog.files[0].duplicate_name);
    assert!(catalog.files[1].duplicate_name);
}

#[::core::prelude::v1::test]
fn catalog_reads_only_optional_profile_presentation_without_resolving_sources() {
    let fixture = Fixture::new();
    let helper = fixture.file("configs/arbitrary-worker.toml");
    std::fs::write(&helper, "include = ['missing-secret-fragment.toml']\nmodel = 'unavailable'\n[profile]\ntitle = '  Worker  '\nentrypoint = false\n").unwrap();
    let main = fixture.file("custom-setup/configs/main.json");
    std::fs::write(
        &main,
        r#"{"profile":{"title":"Custom main"},"models":"not a valid agent models table"}"#,
    )
    .unwrap();
    fixture.file("configs/unknown.toml");
    let catalog = discover(Some(fixture.0.clone()), Some(helper));
    assert!(catalog.problem.is_none());
    assert_eq!(catalog.files.len(), 3);
    let helper = catalog
        .files
        .iter()
        .find(|file| file.name == "arbitrary-worker.toml")
        .unwrap();
    assert_eq!(helper.display_name(), "Worker");
    assert!(!helper.entrypoint);
    assert!(helper.current);
    let main = catalog.files.iter().find(|file| file.path == main).unwrap();
    assert_eq!(main.display_name(), "Custom main");
    assert!(main.entrypoint);
    assert!(
        catalog
            .files
            .iter()
            .find(|file| file.name == "unknown.toml")
            .unwrap()
            .entrypoint
    );
}

#[::core::prelude::v1::test]
fn catalog_marks_colliding_visible_titles_even_for_distinct_filenames() {
    let fixture = Fixture::new();
    for path in ["configs/one.toml", "configs/two.toml"] {
        std::fs::write(fixture.file(path), "[profile]\ntitle = 'My agent'\n").unwrap();
    }
    fixture.file("configs/other.toml");
    let catalog = discover(Some(fixture.0.clone()), None);
    assert_eq!(catalog.files.len(), 3);
    assert_eq!(
        catalog
            .files
            .iter()
            .filter(|file| file.duplicate_name)
            .count(),
        2
    );
    assert!(
        catalog
            .files
            .iter()
            .filter(|file| file.duplicate_name)
            .all(|file| file.display_name() == "My agent")
    );
}

#[::core::prelude::v1::test]
fn oversized_presentation_is_reported_without_hiding_the_config() {
    let fixture = Fixture::new();
    let path = fixture.file("configs/large.toml");
    std::fs::write(path, " ".repeat(MAX_METADATA_BYTES + 1)).unwrap();
    let catalog = discover(Some(fixture.0.clone()), None);
    assert_eq!(catalog.files.len(), 1);
    assert!(catalog.files[0].entrypoint);
    assert!(catalog.problem.is_some());
}

#[::core::prelude::v1::test]
fn catalog_follows_configs_link_once_and_marks_a_canonical_current_alias() {
    let fixture = Fixture::new();
    let actual = fixture.file("setup/configs/config.toml");
    std::os::unix::fs::symlink(fixture.0.join("setup/configs"), fixture.0.join("configs")).unwrap();
    std::os::unix::fs::symlink(&fixture.0, fixture.0.join("setup/loop")).unwrap();
    let catalog = discover(Some(fixture.0.clone()), Some(actual));
    assert!(catalog.problem.is_none());
    assert_eq!(catalog.files.len(), 1);
    assert_eq!(catalog.files[0].relative, "configs/config.toml");
    assert!(catalog.files[0].current);
}

#[::core::prelude::v1::test]
fn catalog_reports_limits_and_keeps_manual_file_selection_available() {
    let fixture = Fixture::new();
    for number in 0..=MAX_CONFIGS {
        fixture.file(&format!("configs/{number:04}.toml"));
    }
    let catalog = discover(Some(fixture.0.clone()), None);
    assert_eq!(catalog.files.len(), MAX_CONFIGS);
    assert!(catalog.problem.unwrap().contains("другой файл"));
    assert!(
        discover(Some(fixture.0.join("not-installed")), None)
            .files
            .is_empty()
    );
    assert!(discover(None, None).problem.is_some());
}

#[::core::prelude::v1::test]
fn catalog_home_uses_the_explicit_setup_before_home_and_xdg_fallback() {
    assert_eq!(
        home_from(
            Some("/custom/setup".into()),
            Some("/home/user".into()),
            Some("/xdg".into())
        ),
        Some("/custom/setup".into())
    );
    assert_eq!(
        home_from(None, Some("/home/user".into()), Some("/xdg".into())),
        Some("/home/user/.config/Proteus-harness".into())
    );
    assert_eq!(
        home_from(None, None, Some("/xdg".into())),
        Some("/xdg/Proteus-harness".into())
    );
}
