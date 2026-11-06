use super::*;
use std::fs;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("build/verification")
            .join(format!("catalog-inventory-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn extension(&self, root: &str, valid: bool) -> PathBuf {
        let path = self.0.join(root).join("terminal");
        fs::create_dir_all(&path).unwrap();
        let text = if valid {
            r#"{"protocol":4,"id":"terminal","title":"Terminal","command":["./run"],"surfaces":{"page":{}}}"#
        } else {
            "{broken manifest"
        };
        fs::write(path.join("extension.json"), text).unwrap();
        path
    }

    fn module(&self, binary: bool) -> PathBuf {
        let root = self.0.join("ready");
        let package = root.join("share/modules/coding-workflow");
        fs::create_dir_all(&package).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        fs::write(root.join("bin/proteus"), "backend").unwrap();
        if binary {
            fs::write(root.join("bin/proteus-coding-workflow-module"), "module").unwrap();
        }
        fs::write(
            package.join("module.json"),
            r#"{
            "id":"coding-workflow", "version":"0.1.0-alpha.3",
            "command":["bin/proteus-coding-workflow-module"],
            "exports":{"workflow":["coding.codex_loop"]}, "sha256":"ignored-by-inventory"
        }"#,
        )
        .unwrap();
        root.join("bin/proteus")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn bundled_modules_use_package_identity_and_require_real_binary() {
    let fixture = Fixture::new();
    let backend = fixture.module(true);
    let inventory = Inventory::read(Some(&backend), &fixture.0.join("none"), None);
    let package = inventory
        .find(
            &OFFICIAL[0].source(),
            Kind::ProcessModule,
            "coding-workflow",
        )
        .unwrap();
    assert_eq!(package.version.as_deref(), Some("0.1.0-alpha.3"));
    assert!(package.problem.is_none());
    assert!(
        inventory
            .find(
                &OFFICIAL[0].source(),
                Kind::ProcessModule,
                "reference-workflow"
            )
            .is_none()
    );
    fs::remove_file(
        backend
            .parent()
            .unwrap()
            .join("proteus-coding-workflow-module"),
    )
    .unwrap();
    let missing = Inventory::read(Some(&backend), &fixture.0.join("none"), None);
    assert!(
        missing
            .find(
                &OFFICIAL[0].source(),
                Kind::ProcessModule,
                "coding-workflow"
            )
            .unwrap()
            .problem
            .is_some()
    );
}

#[test]
fn bundled_extensions_are_present_independently_of_enable_preferences() {
    let fixture = Fixture::new();
    let path = fixture.extension("bundled", true);
    // Inventory intentionally takes no enabled/order preferences: disabled
    // packages remain installed and preserve their state through catalog use.
    let inventory = Inventory::read(None, &fixture.0.join("bundled"), None);
    let package = inventory
        .find(&OFFICIAL[1].source(), Kind::ClientExtension, "terminal")
        .unwrap();
    assert_eq!(package.path, path);
    assert_eq!(package.version, None);
    assert!(package.problem.is_none());
}

#[test]
fn unrelated_source_cannot_adopt_an_installed_id() {
    let fixture = Fixture::new();
    fixture.extension("bundled", true);
    let inventory = Inventory::read(None, &fixture.0.join("bundled"), None);
    let unrelated = Source {
        url: "https://example.invalid/another/extensions.git".into(),
        branch: None,
    };
    assert!(
        inventory
            .find(&unrelated, Kind::ClientExtension, "terminal")
            .is_none()
    );
    assert!(
        inventory
            .find(&OFFICIAL[0].source(), Kind::ClientExtension, "terminal")
            .is_none()
    );
    let normalized = Source {
        url: "https://github.com/Proteus-harness/Proteus-desktop-extensions/".into(),
        branch: None,
    };
    assert!(
        inventory
            .find(&normalized, Kind::ClientExtension, "terminal")
            .is_some()
    );
}

#[test]
fn broken_user_override_is_never_replaced_by_healthy_bundle() {
    let fixture = Fixture::new();
    fixture.extension("bundled", true);
    let user = fixture.extension("user", false);
    let inventory = Inventory::read(
        None,
        &fixture.0.join("bundled"),
        Some(&fixture.0.join("user")),
    );
    let package = inventory
        .find(&OFFICIAL[1].source(), Kind::ClientExtension, "terminal")
        .unwrap();
    assert_eq!(package.path, user);
    assert!(package.problem.is_some());
}

#[test]
fn healthy_user_override_is_not_replaced_by_broken_bundle() {
    let fixture = Fixture::new();
    fixture.extension("bundled", false);
    let user = fixture.extension("user", true);
    let inventory = Inventory::read(
        None,
        &fixture.0.join("bundled"),
        Some(&fixture.0.join("user")),
    );
    let package = inventory
        .find(&OFFICIAL[1].source(), Kind::ClientExtension, "terminal")
        .unwrap();
    assert_eq!(package.path, user);
    assert!(package.problem.is_none());
}

#[test]
fn absent_roots_do_not_invent_installations() {
    let fixture = Fixture::new();
    let inventory = Inventory::read(
        None,
        &fixture.0.join("absent"),
        Some(&fixture.0.join("also-absent")),
    );
    assert!(
        inventory
            .find(&OFFICIAL[1].source(), Kind::ClientExtension, "terminal")
            .is_none()
    );
}
