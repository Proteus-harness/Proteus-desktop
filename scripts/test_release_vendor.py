"""Vendor test harnesses compile the exact checkout without editing manifests."""
import json
import os
import subprocess
import tempfile
from pathlib import Path
import tomllib
import unittest

from test_vendor import manifest


class VendorHarnessTests(unittest.TestCase):
    def temporary(self):
        directory = Path(__file__).resolve().parent.parent / "build/verification/vendor-runner-tests"
        directory.mkdir(parents=True, exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="projection ", dir=directory)
        self.addCleanup(temporary.cleanup)
        return Path(temporary.name)

    def test_nonstandard_lib_path_and_private_lock_are_preserved(self):
        root = self.temporary()
        source = root / "vendor/gpui-fast-wgpu"
        source.mkdir(parents=True)
        original = '[package]\nname="gpui-fast-wgpu"\nversion="0.1.12"\nbuild=false\n[lib]\npath = "src/gpui_wgpu.rs"\n'
        (source / "Cargo.toml").write_text(original)
        (root / "Cargo.toml").write_text('[patch.crates-io]\ngpui-fast-wgpu = { path = "vendor/gpui-fast-wgpu" }\ngpui-base = { path = "vendor/gpui-base" }\n')
        (root / "Cargo.lock").write_text("synthetic production lock")
        target = manifest("gpui-fast-wgpu", root)
        parsed = tomllib.loads(target.read_text())
        self.assertEqual(parsed["lib"]["path"], str(source / "src/gpui_wgpu.rs"))
        self.assertFalse(parsed["package"]["build"])
        self.assertIn("workspace", parsed)
        self.assertEqual(parsed["patch"]["crates-io"]["gpui-base"]["path"], str(root / "vendor/gpui-base"))
        self.assertEqual((source / "Cargo.toml").read_text(), original)
        private_lock = target.with_name("Cargo.lock")
        self.assertEqual(private_lock.read_text(), "synthetic production lock")
        private_lock.write_text("resolved dev dependency graph")
        manifest("gpui-fast-wgpu", root)
        self.assertEqual(private_lock.read_text(), "resolved dev dependency graph")

    def test_source_inputs_and_host_development_profiles_remain_stable(self):
        root = self.temporary()
        source = root / "vendor/gpui-component"
        (source / "src").mkdir(parents=True)
        original = '''[package]
name = "gpui-component"
version = "0.0.0"
build = "build.rs"
[lib]
path = "src/lib.rs"
[profile.dev]
debug = true
[profile.release]
lto = false
'''
        (source / "Cargo.toml").write_text(original)
        for name in ("build.rs", "GHOSTTY.lock"):
            (source / name).write_text("synthetic source input")
        (root / "Cargo.toml").write_text('''[patch.crates-io]
gpui-component = { path = "vendor/gpui-component" }
[profile.dev]
debug = "line-tables-only"
[profile.dev.package."*"]
opt-level = 2
[profile.test]
debug = 1
[profile.release]
lto = "thin"
''')
        (root / "Cargo.lock").write_text("synthetic lock")
        target = manifest("gpui-component", root)
        parsed = tomllib.loads(target.read_text())
        host = tomllib.loads((root / "Cargo.toml").read_text())
        for name in ("dev", "test"):
            self.assertEqual(parsed["profile"][name], host["profile"][name])
        self.assertEqual(parsed["profile"]["release"], {"lto": False})
        self.assertEqual((source / "Cargo.toml").read_text(), original)
        links = {name: target.with_name(name) for name in ("build.rs", "GHOSTTY.lock", "src")}
        for name, link in links.items():
            self.assertTrue(link.is_symlink())
            self.assertEqual(link.resolve(), (source / name).resolve())
        timestamps = {name: link.lstat().st_mtime_ns for name, link in links.items()}
        manifest_time = target.stat().st_mtime_ns
        manifest("gpui-component", root)
        self.assertEqual(target.stat().st_mtime_ns, manifest_time)
        self.assertEqual({name: link.lstat().st_mtime_ns for name, link in links.items()}, timestamps)
        (source / "GHOSTTY.lock").unlink()
        manifest("gpui-component", root)
        self.assertFalse(links["GHOSTTY.lock"].is_symlink())

    def test_offline_cargo_reuses_warm_build_and_observes_original_inputs(self):
        root = self.temporary()
        source = root / "vendor/gpui-component"
        (source / "src").mkdir(parents=True)
        (source / "Cargo.toml").write_text('''[package]
name = "gpui-component"
version = "0.0.0"
edition = "2024"
build = "build.rs"
[lib]
path = "src/lib.rs"
''')
        (source / "src/lib.rs").write_text('''#[test]
fn build_script_read_the_manifest_input() {
    assert_eq!(env!("FIXTURE_VALUE"), std::env::var("EXPECTED").unwrap());
}
''')
        build = source / "build.rs"
        build.write_text('''fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=input.txt");
    let input = std::fs::read_to_string(
        std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("input.txt")
    ).unwrap();
    println!("cargo:rustc-env=FIXTURE_VALUE={}", input.trim());
}
''')
        data = source / "input.txt"
        data.write_text("first")
        (root / "Cargo.toml").write_text('''[patch.crates-io]
gpui-component = { path = "vendor/gpui-component" }
[profile.dev]
debug = "line-tables-only"
[profile.dev.package."*"]
opt-level = 2
''')
        (root / "Cargo.lock").write_text("version = 4\n")

        def run(expected):
            target = manifest("gpui-component", root)
            result = subprocess.run(
                ["cargo", "test", "--offline", "--manifest-path", str(target),
                 "--target-dir", str(root / "cargo-target"), "--lib", "--message-format=json"],
                cwd=root, env=dict(os.environ, EXPECTED=expected),
                text=True, capture_output=True, timeout=30,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            messages = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
            artifacts = [message for message in messages if message.get("reason") == "compiler-artifact"
                         and message["target"]["kind"] == ["lib"]]
            self.assertEqual(len(artifacts), 1, result.stdout)
            self.assertEqual(artifacts[0]["profile"]["opt_level"], "0")
            return artifacts[0]["fresh"]

        self.assertFalse(run("first"))
        self.assertTrue(run("first"))
        data.write_text("second")
        self.assertFalse(run("second"))
        self.assertTrue(run("second"))
        build.write_text(build.read_text().replace("input.trim()", 'format!("{}-script", input.trim())'))
        self.assertFalse(run("second-script"))
