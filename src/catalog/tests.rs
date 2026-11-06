use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use super::{schema::Kind, *};
use crate::{packages::Contents, settings::Draft};

mod plugin_identity;
mod source_download;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from("/tmp/opencode")
            .join(format!("proteus-catalog-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
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
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/catalog")
}
fn source(path: &Path) -> Source {
    Source {
        url: path.to_str().unwrap().to_string(),
        branch: None,
    }
}
fn git_at(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn commit(path: &Path) {
    git_at(path, &["add", "."]);
    git_at(
        path,
        &[
            "-c",
            "user.name=Catalog Test",
            "-c",
            "user.email=catalog@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "fixture",
        ],
    );
}

#[test]
fn git_refresh_keeps_last_good_checkout_and_reports_bad_entries() {
    let fixture = Fixture::new();
    let repo = Contents::folder(&example())
        .unwrap()
        .install(&fixture.0, "repo", false)
        .unwrap();
    git_at(&repo, &["init", "-q", "-b", "main"]);
    commit(&repo);
    let source = source(&repo);
    let cache = fixture.locations().cache;
    let first = git::refresh(&cache, &source).unwrap();
    assert_eq!(first.entries.len(), 4);
    // Only the index is downloaded; an entry's files come when asked for.
    let demo = first.entries[0].path.join("README.md");
    assert!(!first.root.join(&demo).exists());
    let readme = git::with_files(&cache, &source, &[demo.clone()], || {
        schema::readme(&first, &first.entries[0])
    })
    .unwrap();
    assert!(readme.is_some());
    let mut index: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(repo.join(schema::INDEX)).unwrap()).unwrap();
    index["entries"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"id":"broken","kind":"unknown"}));
    index["entries"][0]["version"] = "2.0.0".into();
    fs::write(repo.join(schema::INDEX), index.to_string()).unwrap();
    // Removed files must disappear on refresh as well, not survive in git's index.
    fs::remove_file(repo.join("client/demo/README.md")).unwrap();
    commit(&repo);
    let second = git::refresh(&cache, &source).unwrap();
    assert_eq!(second.entries[0].version, "2.0.0");
    assert_eq!(second.problems.len(), 1);
    assert!(
        git::with_files(&cache, &source, &[demo], || schema::readme(
            &second,
            &second.entries[0]
        ))
        .unwrap()
        .is_none()
    );
    fs::write(repo.join(schema::INDEX), "{bad").unwrap();
    commit(&repo);
    assert!(git::refresh(&cache, &source).is_err());
    assert_eq!(
        git::cached(&cache, &source).unwrap().entries[0].version,
        "2.0.0"
    );
    let missing = Source {
        branch: Some("missing".into()),
        ..source.clone()
    };
    assert!(git::refresh(&cache, &missing).is_err());
}

#[test]
fn installation_binding_update_and_removal_use_the_existing_formats() {
    let fixture = Fixture::new();
    let locations = fixture.locations();
    let catalog = schema::read(&example(), "example", &Default::default()).unwrap();
    let source = source(&example());
    let profile = fixture.0.join("profile.toml");
    let mut draft = Draft::default();
    for entry in &catalog.entries {
        let installed =
            install::prepare(&locations, &source, &catalog, entry, None, &profile, &[]).unwrap();
        install::connect(&mut draft, &installed, None).unwrap();
        assert!(install::connected(&installed, &draft));
        let mut newer = entry.clone();
        newer.version = "2.0.0".into();
        let updated = install::prepare(
            &locations,
            &source,
            &catalog,
            &newer,
            Some(&installed),
            &profile,
            &[],
        )
        .unwrap();
        install::connect(&mut draft, &updated, Some(&installed)).unwrap();
        if entry.kind == Kind::AgentPlugin {
            let Target::Folder { path: old } = &installed.target else {
                panic!()
            };
            let Target::Folder { path: new } = &updated.target else {
                panic!()
            };
            assert_ne!(old, new);
            assert!(
                old.join("plugin.json").is_file(),
                "saved versions stay intact"
            );
            assert_eq!(draft.addons.addons.plugins.len(), 1);
            // A discarded update leaves its files, not a durable receipt.
            // Retrying against the saved receipt must not collide with them.
            let retried = install::prepare(
                &locations,
                &source,
                &catalog,
                &newer,
                Some(&installed),
                &profile,
                &[],
            )
            .unwrap();
            assert_ne!(retried.target, updated.target);
            assert!(old.join("plugin.json").is_file());
        }
        install::disconnect(&mut draft, &updated).unwrap();
        install::remove_files(&locations, &updated).unwrap();
        if entry.kind == Kind::AgentPlugin {
            let Target::Folder { path } = &updated.target else {
                panic!()
            };
            assert!(path.is_dir());
        } else {
            assert!(!install::connected(&updated, &draft));
        }
    }
    let extension = &catalog.entries[0];
    assert!(
        install::prepare(
            &locations,
            &source,
            &catalog,
            extension,
            None,
            &profile,
            &[extension.id.clone()]
        )
        .is_err()
    );
}

fn plugin_receipt(path: impl Into<PathBuf>) -> Installed {
    Installed {
        source: source(Path::new("catalog")),
        entry: "plugin".into(),
        version: "1.0.0".into(),
        kind: Kind::AgentPlugin,
        profile: Some("profile.toml".into()),
        target: Target::Folder { path: path.into() },
    }
}

#[test]
fn plugin_catalog_updates_preserve_the_profile_switch() {
    let fixture = Fixture::new();
    let old = fixture.0.join("plugin-v1");
    let newer = fixture.0.join("plugin-v2");
    fs::create_dir_all(&old).unwrap();
    let installed = plugin_receipt(old.clone());
    for enabled in [false, true] {
        for path in [&old, &newer] {
            let mut draft = Draft::default();
            install::connect(&mut draft, &installed, None).unwrap();
            draft.set_package(&old, enabled);
            draft.add_package("unrelated".into());
            draft.set_package(Path::new("unrelated"), false);
            let baseline = draft.clone();
            let updated = plugin_receipt(path);

            install::connect(&mut draft, &updated, Some(&installed)).unwrap();

            let plugins = &draft.addons.addons.plugins;
            assert_eq!(plugins.len(), 2);
            let unrelated = plugins
                .iter()
                .find(|plugin| plugin.path == Path::new("unrelated"))
                .unwrap();
            assert!(!unrelated.enabled);
            let current = plugins.iter().find(|plugin| plugin.path == *path).unwrap();
            assert_eq!(current.enabled, enabled);
            assert!(install::connected(&updated, &draft));

            if path != &old {
                assert!(!install::connected(&installed, &draft));
                let saved = draft.changes(&baseline).addon_settings.unwrap();
                assert_eq!(saved.addons.plugins, draft.addons.addons.plugins);
            }

            let connected = draft.clone();
            install::connect(&mut draft, &updated, Some(&updated)).unwrap();
            assert_eq!(draft, connected, "retrying an update keeps the switch");
        }
    }
}

#[test]
fn plugin_catalog_installs_without_a_current_binding_start_enabled() {
    let installed = plugin_receipt("plugin-v1");
    let updated = plugin_receipt("plugin-v2");
    for previous in [None, Some(&installed)] {
        let mut draft = Draft::default();
        draft.add_package("unrelated".into());
        draft.set_package(Path::new("unrelated"), false);

        install::connect(&mut draft, &updated, previous).unwrap();

        let plugins = &draft.addons.addons.plugins;
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].path, Path::new("unrelated"));
        assert!(!plugins[0].enabled);
        assert_eq!(plugins[1].path, Path::new("plugin-v2"));
        assert!(plugins[1].enabled);
    }
}

#[test]
fn mcp_conflicts_dont_overwrite_manual_edits_and_receipts_are_profile_scoped() {
    let fixture = Fixture::new();
    let catalog = schema::read(&example(), "example", &Default::default()).unwrap();
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.kind == Kind::McpServer)
        .unwrap();
    let source = source(&example());
    let profile = fixture.0.join("a.toml");
    let installed = install::prepare(
        &fixture.locations(),
        &source,
        &catalog,
        entry,
        None,
        &profile,
        &[],
    )
    .unwrap();
    let mut prefs = Preferences::default();
    prefs.remember(installed.clone());
    assert!(install::receipt(&prefs.installed, &source, entry, &profile).is_some());
    assert!(
        install::receipt(&prefs.installed, &source, entry, &fixture.0.join("b.toml")).is_none()
    );
    let mut draft = Draft::default();
    install::connect(&mut draft, &installed, None).unwrap();
    let baseline = draft.clone();
    assert!(install::connect(&mut draft, &installed, None).is_err());
    assert_eq!(draft, baseline);
    draft.addons.mcp_servers[0].command = "edited".into();
    let edited = draft.clone();
    assert!(install::connect(&mut draft, &installed, Some(&installed)).is_err());
    assert!(install::disconnect(&mut draft, &installed).is_err());
    assert_eq!(draft, edited);
    // Receipts persist through discard: reconnecting after discard remains possible.
    draft = Draft::default();
    install::connect(&mut draft, &installed, Some(&installed)).unwrap();
    assert_eq!(draft, baseline);
}

#[test]
fn plugin_zip_and_invalid_catalog_paths_share_checked_package_io() {
    use std::io::Write;
    let fixture = Fixture::new();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("plugin.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer
        .write_all(&fs::read(example().join("plugins/review/plugin.json")).unwrap())
        .unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    let archive = fixture.0.join("plugin.zip");
    fs::write(&archive, bytes).unwrap();
    let imported = install::import_zip(&fixture.locations(), &archive, vec![], None).unwrap();
    assert!(imported.path.join("plugin.json").is_file());
    assert!(schema::relative(Path::new("../outside")).is_err());
    assert!(schema::relative(Path::new("/absolute")).is_err());
    assert!(schema::relative(Path::new("a/.git/config")).is_err());
    let mut catalog = schema::read(&example(), "example", &Default::default()).unwrap();
    catalog.entries[0].path = "../outside".into();
    assert!(schema::directory(&catalog.root, &catalog.entries[0]).is_err());
    let mut broken = Source {
        url: "-x".into(),
        branch: None,
    };
    assert!(git::validate(&broken).is_err());
    broken.url = "x".into();
    broken.branch = Some("--bad".into());
    assert!(git::validate(&broken).is_err());
}

#[test]
fn repository_without_index_lists_agent_skills_as_installable_entries() {
    let fixture = Fixture::new();
    let root = fixture.0.join("repo");
    let skill = |dir: &str, front: &str| {
        let path = root.join(dir);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!("---\n{front}\n---\n\n# Body\n"),
        )
        .unwrap();
    };
    skill("skills/.curated/pdf", "name: pdf\ndescription: Read PDFs");
    // Nested folders belong to the enclosing skill.
    skill(
        "skills/.curated/pdf/forms",
        "name: forms\ndescription: Part of pdf",
    );
    skill("skills/.curated/docs", "name: docs\ndescription: Curated");
    skill("skills/.system/docs", "name: docs\ndescription: System");
    skill("template", "name: template-skill\ndescription: Mismatch");
    skill("tools/lint", "name: lint\ndescription: >\n  Folded\n  text");
    fs::write(root.join("skills/.curated/pdf/forms/form.txt"), "form").unwrap();
    git_at(&root, &["init", "-q", "-b", "main"]);
    commit(&root);
    let source = source(&root);
    let locations = fixture.locations();
    let cache = &locations.cache;
    let catalog = git::refresh(cache, &source).unwrap();
    assert_eq!(catalog.title, source.name());
    let ids = catalog
        .entries
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["pdf", "lint"]);
    assert_eq!(catalog.problems.len(), 2, "{:?}", catalog.problems);
    let categories = catalog
        .categories
        .iter()
        .map(|category| category.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(categories, ["skills/.curated", "tools"]);
    let (pdf, lint) = (&catalog.entries[0], &catalog.entries[1]);
    assert_eq!(
        (pdf.kind, lint.description.as_str()),
        (Kind::Skill, "Folded text")
    );
    // Listing takes SKILL.md files only.
    assert!(!catalog.root.join(&pdf.path).join("forms/form.txt").exists());
    let files = [pdf.path.join("README.md"), pdf.path.join("SKILL.md")];
    assert_eq!(
        git::with_files(cache, &source, &files, || schema::readme(&catalog, pdf))
            .unwrap()
            .as_deref(),
        Some("# Body")
    );

    let installed = git::with_files(cache, &source, &[pdf.path.clone()], || {
        install::prepare(&locations, &source, &catalog, pdf, None, &fixture.0, &[])
    })
    .unwrap();
    assert!(locations.skills.join("pdf/forms/form.txt").is_file());
    fs::write(root.join("skills/.curated/pdf/forms/extra.md"), "more").unwrap();
    commit(&root);
    let changed = git::refresh(cache, &source).unwrap();
    assert_ne!(changed.entries[0].version, installed.version);
    assert_eq!(changed.entries[1].version, lint.version);
    for entry in fs::read_dir(&root).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().unwrap() != ".git" {
            fs::remove_dir_all(&path)
                .or_else(|_| fs::remove_file(&path))
                .unwrap();
        }
    }
    fs::write(root.join("README.md"), "nothing").unwrap();
    commit(&root);
    assert!(git::refresh(cache, &source).is_err());

    for url in [
        "https://github.com/owner/skills.git/",
        "git@github.com:owner/skills",
    ] {
        let source = Source {
            url: url.into(),
            branch: None,
        };
        assert_eq!(source.name(), "owner/skills");
    }
}

#[test]
fn extension_preflight_checks_staged_files_before_installing_or_replacing() {
    use crate::declarative::manifest;

    let fixture = Fixture::new();
    let repo = Contents::folder(&example())
        .unwrap()
        .install(&fixture.0, "repo", false)
        .unwrap();
    let catalog = schema::read(&repo, "example", &Default::default()).unwrap();
    let entry = catalog
        .entries
        .iter()
        .find(|entry| entry.kind == Kind::ClientExtension)
        .unwrap();
    let package = repo.join(&entry.path);
    let manifest_path = package.join(manifest::MANIFEST);
    let mut value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    value["command"] = serde_json::json!(["proteus-preflight-must-not-run"]);
    value["required_features"] = serde_json::json!(["manifest.agent_instructions.v1"]);
    value["agent_instructions"] = serde_json::json!([{"skill": "guide", "path": "skills/guide"}]);
    let original = value.to_string();
    fs::write(&manifest_path, &original).unwrap();

    let source = source(&repo);
    let locations = fixture.locations();
    let profile = fixture.0.join("profile.toml");
    assert!(install::prepare(&locations, &source, &catalog, entry, None, &profile, &[]).is_err());
    assert_eq!(fs::read_dir(&locations.extensions).unwrap().count(), 0);

    let skill = package.join("skills/guide/SKILL.md");
    fs::create_dir_all(skill.parent().unwrap()).unwrap();
    fs::write(&skill, "# Original").unwrap();
    let previous =
        install::prepare(&locations, &source, &catalog, entry, None, &profile, &[]).unwrap();
    let destination = locations.extensions.join(&entry.id);
    assert_eq!(manifest::load(&destination).unwrap().id, entry.id);
    // The catalog folder is named demo, not catalog-demo. Validation must use
    // the staged final id, rather than the source or hidden staging folder name.
    assert_ne!(package.file_name(), destination.file_name());

    let mut newer = entry.clone();
    newer.version = "2.0.0".into();
    value["title"] = serde_json::json!("Updated");
    fs::write(&manifest_path, value.to_string()).unwrap();
    fs::remove_file(&skill).unwrap();
    assert!(
        install::prepare(
            &locations,
            &source,
            &catalog,
            &newer,
            Some(&previous),
            &profile,
            &[entry.id.clone()],
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(destination.join(manifest::MANIFEST)).unwrap(),
        original
    );
    assert_eq!(
        fs::read_to_string(destination.join("skills/guide/SKILL.md")).unwrap(),
        "# Original"
    );
    assert!(manifest::load(&destination).is_ok());
    assert_eq!(fs::read_dir(&locations.extensions).unwrap().count(), 1);

    fs::write(&skill, "# Updated").unwrap();
    let updated = install::prepare(
        &locations,
        &source,
        &catalog,
        &newer,
        Some(&previous),
        &profile,
        &[entry.id.clone()],
    )
    .unwrap();
    assert_eq!(updated.target, previous.target);
    assert_eq!(updated.version, "2.0.0");
    assert_eq!(manifest::load(&destination).unwrap().title, "Updated");
    assert_eq!(
        fs::read_to_string(destination.join("skills/guide/SKILL.md")).unwrap(),
        "# Updated"
    );
    assert_eq!(fs::read_dir(&locations.extensions).unwrap().count(), 1);
}
