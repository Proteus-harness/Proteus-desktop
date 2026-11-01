use super::*;

fn declaration() -> serde_json::Value {
    serde_json::json!({
        "protocol": 4, "id": "renderer", "title": "Renderer", "command": ["run"],
        "surfaces": {"content": {"languages": ["custom"]}},
        "required_features": ["manifest.agent_instructions.v1"],
        "agent_instructions": [{"skill": "org.example/visualize", "path": "skills/visualize"}]
    })
}

#[test]
fn packaged_skill_links_are_optional_and_independent_of_runtime_dependencies() {
    let mut value = declaration();
    let manifest = parse(&value.to_string()).unwrap();
    assert_eq!(
        manifest.agent_instructions[0].skill,
        "org.example/visualize"
    );
    assert_eq!(
        manifest.agent_instructions[0].path,
        Path::new("skills/visualize")
    );
    assert!(manifest.dependencies.tools.is_empty());
    value.as_object_mut().unwrap().remove("agent_instructions");
    assert!(
        parse(&value.to_string())
            .unwrap()
            .agent_instructions
            .is_empty()
    );
}

#[test]
fn packaged_skill_links_reject_ambiguous_identifiers_and_paths() {
    for skill in ["", " two", "two words", "two\twords", "two\u{0000}words"] {
        let mut value = declaration();
        value["agent_instructions"][0]["skill"] = serde_json::json!(skill);
        assert!(parse(&value.to_string()).is_err(), "skill: {skill:?}");
    }
    for path in [
        "",
        ".",
        "..",
        "/skills/visualize",
        "skills/../visualize",
        "skills/./visualize",
        "./skills/visualize",
        "skills//visualize",
        "skills/visualize/",
        "C:/visualize",
        "skills\\visualize",
        "skills/visualize\u{0000}",
    ] {
        let mut value = declaration();
        value["agent_instructions"][0]["path"] = serde_json::json!(path);
        assert!(parse(&value.to_string()).is_err(), "path: {path:?}");
    }
    for duplicate in [
        serde_json::json!({"skill": "org.example/visualize", "path": "skills/other"}),
        serde_json::json!({"skill": "other", "path": "skills/visualize"}),
    ] {
        let mut value = declaration();
        value["agent_instructions"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(parse(&value.to_string()).is_err());
    }
    let mut value = declaration();
    value["agent_instructions"][0]["install"] = serde_json::json!(true);
    assert!(parse(&value.to_string()).is_err());
}

struct Package(PathBuf);

impl Package {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("proteus-skill-{}", uuid::Uuid::new_v4()));
        let dir = root.join("renderer");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MANIFEST), declaration().to_string()).unwrap();
        Self(dir)
    }

    fn skill_dir(&self) -> PathBuf {
        self.0.join("skills/visualize")
    }
}

impl Drop for Package {
    fn drop(&mut self) {
        std::fs::remove_dir_all(self.0.parent().unwrap()).unwrap();
    }
}

#[test]
fn discovery_requires_a_packaged_regular_skill_file_without_reading_its_contents() {
    let package = Package::new();
    assert!(load(&package.0).is_err());
    std::fs::create_dir_all(package.skill_dir()).unwrap();
    assert!(load(&package.0).is_err());
    let skill_file = package.skill_dir().join("SKILL.md");
    std::fs::create_dir(&skill_file).unwrap();
    assert!(load(&package.0).is_err());
    std::fs::remove_dir(&skill_file).unwrap();
    // Discovery checks the link, not frontmatter or instructions; skill handling
    // belongs to the agent configuration and never starts the renderer process.
    std::fs::write(skill_file, "# Instructions").unwrap();
    assert!(load(&package.0).is_ok());
    let (installed, problems) = discover(package.0.parent().unwrap());
    assert_eq!(installed.len(), 1);
    assert!(problems.is_empty());
}

#[cfg(unix)]
#[test]
fn discovery_rejects_external_symlinks_and_duplicate_resolved_directories() {
    use std::os::unix::fs::symlink;

    let package = Package::new();
    let outside = package.0.parent().unwrap().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("SKILL.md"), "# Outside").unwrap();
    std::fs::create_dir(package.0.join("skills")).unwrap();
    symlink(&outside, package.skill_dir()).unwrap();
    assert!(load(&package.0).is_err());
    std::fs::remove_file(package.skill_dir()).unwrap();
    std::fs::create_dir(package.skill_dir()).unwrap();
    symlink(
        outside.join("SKILL.md"),
        package.skill_dir().join("SKILL.md"),
    )
    .unwrap();
    assert!(load(&package.0).is_err());
    std::fs::remove_file(package.skill_dir().join("SKILL.md")).unwrap();
    std::fs::write(package.skill_dir().join("SKILL.md"), "# Inside").unwrap();
    symlink(package.skill_dir(), package.0.join("skills/alias")).unwrap();
    let mut value = declaration();
    value["agent_instructions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"skill": "alias", "path": "skills/alias"}));
    std::fs::write(package.0.join(MANIFEST), value.to_string()).unwrap();
    assert!(load(&package.0).is_err());
}
