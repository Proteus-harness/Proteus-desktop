use std::io::Write;

use super::*;

fn manifest(name: &str, version: &str) -> String {
    serde_json::json!({
        "$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
        "name":name,
        "version":version
    })
    .to_string()
}

fn package(root: &Path, name: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(root.join("plugin.json"), manifest(name, "1.0.0")).unwrap();
}

fn archive(fixture: &Fixture, name: &str, version: &str) -> PathBuf {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file("plugin.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer
        .write_all(manifest(name, version).as_bytes())
        .unwrap();
    let path = fixture.0.join(format!("{name}-{version}.zip"));
    fs::write(&path, writer.finish().unwrap().into_inner()).unwrap();
    path
}

fn import_zip(
    locations: &Locations,
    archive: &Path,
    packages: Vec<proteus_contracts::domain::AgentPluginConfig>,
) -> anyhow::Result<install::ZipImport> {
    install::import_zip(locations, archive, packages, None)
}

#[test]
fn catalog_updates_and_rollback_keep_the_original_state_address() {
    let fixture = Fixture::new();
    let first = fixture.0.join("from-catalog-v1");
    let second = fixture.0.join("from-catalog-v2");
    let third = fixture.0.join("from-catalog-v3");
    for path in [&first, &second, &third] {
        package(path, "stateful");
    }
    let original_key = first.canonicalize().unwrap().to_string_lossy().into_owned();
    for explicit in [None, Some("installation-owner-key".to_string())] {
        let mut draft = Draft::default();
        draft.add_package(first.clone());
        draft.addons.addons.plugins[0].enabled = false;
        draft.addons.addons.plugins[0].data_key = explicit.clone();
        let saved = draft.clone();
        let expected = explicit.unwrap_or_else(|| original_key.clone());
        let v1 = plugin_receipt(first.clone());
        let v2 = plugin_receipt(second.clone());
        let v3 = plugin_receipt(third.clone());

        install::connect(&mut draft, &v2, Some(&v1)).unwrap();
        assert_eq!(draft.addons.addons.plugins.len(), 1);
        assert!(!draft.addons.addons.plugins[0].enabled);
        assert_eq!(
            draft.addons.addons.plugins[0].data_key.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            draft.changes(&saved).addon_settings.unwrap().addons.plugins,
            draft.addons.addons.plugins
        );
        install::connect(&mut draft, &v3, Some(&v2)).unwrap();
        install::connect(&mut draft, &v1, Some(&v3)).unwrap();
        assert_eq!(draft.addons.addons.plugins[0].path, first);
        assert_eq!(
            draft.addons.addons.plugins[0].data_key.as_deref(),
            Some(expected.as_str())
        );
        assert!(!draft.addons.addons.plugins[0].enabled);
        assert!(second.join("plugin.json").is_file());
        // Losing a client receipt does not reset an already bound folder's key.
        let rolled_back = draft.clone();
        install::connect(&mut draft, &v1, None).unwrap();
        assert_eq!(draft, rolled_back);

        // Another profile creates its own binding, without borrowing this one.
        let mut other = Draft::default();
        install::connect(&mut other, &v2, Some(&v1)).unwrap();
        assert_eq!(other.addons.addons.plugins[0].data_key, None);
        assert!(other.addons.addons.plugins[0].enabled);
    }
}

#[test]
fn zip_reimport_requires_replacement_and_keeps_the_switch_and_data_key() {
    let fixture = Fixture::new();
    let locations = fixture.locations();
    let first_zip = archive(&fixture, "stateful", "1.0.0");
    let newer_zip = archive(&fixture, "stateful", "2.0.0");
    let mut draft = Draft::default();
    let first = import_zip(&locations, &first_zip, vec![]).unwrap();
    assert!(first.previous.is_none());
    let first_path = first.path.clone();
    first.connect(&mut draft).unwrap();
    draft.set_package(&first_path, false);
    let original_key = first_path
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();

    for archive in [&first_zip, &newer_zip] {
        let import = import_zip(&locations, archive, draft.addons.addons.plugins.clone()).unwrap();
        assert_eq!(import.name, "stateful");
        assert_eq!(
            import.previous.as_ref(),
            draft.addons.addons.plugins.first()
        );
        assert_ne!(import.path, draft.addons.addons.plugins[0].path);
        import.connect(&mut draft).unwrap();
        assert_eq!(draft.addons.addons.plugins.len(), 1);
        assert!(!draft.addons.addons.plugins[0].enabled);
        assert_eq!(
            draft.addons.addons.plugins[0].data_key.as_deref(),
            Some(original_key.as_str())
        );
        assert!(first_path.join("plugin.json").is_file());
    }
}

#[test]
fn zip_collision_from_another_source_requires_the_same_explicit_choice() {
    let fixture = Fixture::new();
    let configured = fixture.0.join("another-source/plugin");
    package(&configured, "stateful");
    let mut draft = Draft::default();
    draft.add_package(configured.clone());
    draft.addons.addons.plugins[0].data_key = Some("existing-data".into());
    draft.set_package(&configured, false);
    let baseline = draft.clone();
    let zip = archive(&fixture, "stateful", "2.0.0");
    let import = import_zip(
        &fixture.locations(),
        &zip,
        draft.addons.addons.plugins.clone(),
    )
    .unwrap();
    assert_eq!(
        import.previous.as_ref(),
        baseline.addons.addons.plugins.first()
    );
    let staged = import.path.clone();
    // Cancel is represented by dropping the preparation without rebinding.
    drop(import);
    assert_eq!(draft, baseline);
    assert!(configured.join("plugin.json").is_file());
    assert!(staged.join("plugin.json").is_file());

    let import = import_zip(
        &fixture.locations(),
        &zip,
        draft.addons.addons.plugins.clone(),
    )
    .unwrap();
    import.connect(&mut draft).unwrap();
    assert_eq!(draft.addons.addons.plugins.len(), 1);
    assert_eq!(
        draft.addons.addons.plugins[0].data_key.as_deref(),
        Some("existing-data")
    );
    assert!(!draft.addons.addons.plugins[0].enabled);
    assert!(configured.join("plugin.json").is_file());
}

#[test]
fn zip_result_rejects_a_changed_package_list_and_preserves_other_edits() {
    let fixture = Fixture::new();
    let zip = archive(&fixture, "stateful", "1.0.0");
    let mut draft = Draft::default();
    let initial = import_zip(&fixture.locations(), &zip, vec![]).unwrap();
    let original = initial.path.clone();
    initial.connect(&mut draft).unwrap();
    let stale = import_zip(
        &fixture.locations(),
        &zip,
        draft.addons.addons.plugins.clone(),
    )
    .unwrap();
    draft.set_package(&original, false);
    let edited = draft.clone();
    assert!(stale.connect(&mut draft).is_err());
    assert_eq!(draft, edited);

    let fresh = import_zip(
        &fixture.locations(),
        &zip,
        draft.addons.addons.plugins.clone(),
    )
    .unwrap();
    draft.provider = Some("newer-provider".into());
    fresh.connect(&mut draft).unwrap();
    assert_eq!(draft.provider.as_deref(), Some("newer-provider"));
    assert!(!draft.addons.addons.plugins[0].enabled);
    assert_eq!(draft.addons.addons.plugins.len(), 1);
}

#[test]
fn zip_cannot_choose_between_multiple_existing_names() {
    let fixture = Fixture::new();
    let mut draft = Draft::default();
    for folder in ["source-a", "source-b"] {
        let path = fixture.0.join(folder);
        package(&path, "stateful");
        draft.add_package(path);
    }
    let baseline = draft.clone();
    let zip = archive(&fixture, "stateful", "2.0.0");
    assert!(
        import_zip(
            &fixture.locations(),
            &zip,
            draft.addons.addons.plugins.clone()
        )
        .is_err()
    );
    assert_eq!(draft, baseline);
    assert!(!fixture.locations().plugins.exists());
}

#[test]
fn zip_matches_a_relative_binding_in_the_core_workspace() {
    let fixture = Fixture::new();
    let relative = PathBuf::from("plugins/stateful");
    let root = fixture.0.join(&relative);
    package(&root, "stateful");
    let mut draft = Draft::default();
    draft.add_package(relative.clone());
    draft.set_package(&relative, false);
    let archive = archive(&fixture, "stateful", "2.0.0");
    let packages = draft.addons.addons.plugins.clone();
    assert!(import_zip(&fixture.locations(), &archive, packages.clone()).is_err());
    let imported =
        install::import_zip(&fixture.locations(), &archive, packages, Some(&fixture.0)).unwrap();
    assert_eq!(imported.previous.as_ref().unwrap().path, relative);
    imported.connect(&mut draft).unwrap();
    assert_eq!(draft.addons.addons.plugins.len(), 1);
    assert!(!draft.addons.addons.plugins[0].enabled);
    assert_eq!(
        draft.addons.addons.plugins[0].data_key.as_deref(),
        Some(root.canonicalize().unwrap().to_string_lossy().as_ref())
    );
}

#[test]
fn a_missing_old_root_needs_an_explicit_data_key_before_rebinding() {
    let fixture = Fixture::new();
    let old = fixture.0.join("missing");
    let new = fixture.0.join("new");
    let previous = plugin_receipt(old.clone());
    let updated = plugin_receipt(new.clone());
    let mut draft = Draft::default();
    draft.add_package(old);
    let baseline = draft.clone();
    assert!(install::connect(&mut draft, &updated, Some(&previous)).is_err());
    assert_eq!(draft, baseline);
    draft.addons.addons.plugins[0].data_key = Some("known-state".into());
    install::connect(&mut draft, &updated, Some(&previous)).unwrap();
    assert_eq!(draft.addons.addons.plugins[0].path, new);
    assert_eq!(
        draft.addons.addons.plugins[0].data_key.as_deref(),
        Some("known-state")
    );
}
