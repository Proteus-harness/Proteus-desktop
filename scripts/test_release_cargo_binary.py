"""Stdlib-only regression tests; no Cargo, npm, builds or product binaries run."""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

SCRIPTS = Path(__file__).resolve().parent


def load_file(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


cargo_binary = load_file("cargo_binary", SCRIPTS / "cargo_binary.py")


class CargoBinaryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="portable artifact tests ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.cwd = self.root / "checkout with spaces"
        self.cwd.mkdir()
        self.manifest = self.cwd / "crates/core/Cargo.toml"
        self.manifest.parent.mkdir(parents=True)
        self.manifest.write_text("# inert fixture\n")
        self.output = self.root / "cache with spaces/x86_64-target/release/proteus"
        self.output.parent.mkdir(parents=True)
        self.output.write_text("inert fixture, never executed\n")

    def artifact(self, **changes):
        record = {
            "reason": "compiler-artifact",
            "package_id": "path+file:///inert#proteus-core@0.1.0",
            "manifest_path": str(self.manifest),
            "target": {"name": "proteus", "kind": ["bin"]},
            "profile": {"test": False},
            "executable": str(self.output),
            "fresh": False,
        }
        record.update(changes)
        return record

    def invoke(self, records, status=0, raw=None, finished=True):
        if finished:
            records = [*records, {"reason": "build-finished", "success": True}]
        stdout = raw if raw is not None else "\n".join(map(json.dumps, records)) + "\n"
        result = subprocess.CompletedProcess(["cargo"], status, stdout)
        self.run = Mock(return_value=result)
        self.stderr = io.StringIO()
        with patch.object(cargo_binary.subprocess, "run", self.run):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(self.stderr):
                return cargo_binary.build_binary(
                    self.cwd, "proteus-core", "proteus", ["--release"], self.manifest,
                )

    def test_exact_artifact_path_not_stale_default_target(self):
        stale = self.cwd / "target/release/proteus"
        stale.parent.mkdir(parents=True)
        stale.write_text("stale, never executed")
        self.assertEqual(self.invoke([self.artifact()]), self.output)
        self.assertEqual(stale.read_text(), "stale, never executed")

    def test_fresh_artifact_is_valid(self):
        self.assertEqual(self.invoke([self.artifact(fresh=True)]), self.output)

    def test_relative_artifact_paths_use_cargo_cwd(self):
        output = self.cwd / "relative cache/release/proteus"
        output.parent.mkdir(parents=True)
        output.write_text("inert")
        self.assertEqual(self.invoke([self.artifact(
            manifest_path="crates/core/Cargo.toml",
            executable="relative cache/release/proteus",
        )]), output)

    def test_target_environment_and_release_flags_are_preserved(self):
        with patch.dict(os.environ, {"CARGO_TARGET_DIR": "cache with spaces"}):
            self.invoke([self.artifact()])
            self.assertEqual(os.environ["CARGO_TARGET_DIR"], "cache with spaces")
        args, kwargs = self.run.call_args
        self.assertEqual(args[0], [
            "cargo", "build", "--locked", "--release", "-p", "proteus-core",
            "--bin", "proteus", "--message-format=json-render-diagnostics",
        ])
        self.assertEqual(kwargs, {
            "cwd": self.cwd, "stdout": subprocess.PIPE, "text": True, "check": False,
        })

    def test_wrong_package_same_binary_name_rejected(self):
        with self.assertRaisesRegex(SystemExit, "exactly one"):
            self.invoke([self.artifact(
                package_id="path+file:///other#other-package@0.1.0",
                manifest_path=str(self.cwd / "other/Cargo.toml"),
            )])

    def test_wrong_name_kind_or_test_artifact_rejected(self):
        changes = [
            {"target": {"name": "another", "kind": ["bin"]}},
            {"target": {"name": "proteus", "kind": ["lib"]}},
            {"target": {"name": "proteus", "kind": ["custom-build"]}},
            {"profile": {"test": True}},
        ]
        for change in changes:
            with self.subTest(change=change), self.assertRaisesRegex(SystemExit, "exactly one"):
                self.invoke([self.artifact(**change)])

    def test_unrelated_artifact_does_not_override_selected_binary(self):
        wrong = self.artifact(manifest_path=str(self.cwd / "other/Cargo.toml"))
        self.assertEqual(self.invoke([wrong, self.artifact(), wrong]), self.output)

    def test_missing_or_empty_executable_rejected(self):
        for executable in (None, "", 42):
            with self.subTest(executable=executable), self.assertRaisesRegex(SystemExit, "exactly one"):
                self.invoke([self.artifact(executable=executable)])

    def test_no_artifact_rejected(self):
        with self.assertRaisesRegex(SystemExit, "exactly one"):
            self.invoke([{"reason": "build-finished", "success": True}])

    def test_duplicate_artifacts_rejected_even_if_identical(self):
        with self.assertRaisesRegex(SystemExit, "exactly one"):
            self.invoke([self.artifact(), self.artifact()])

    def test_missing_output_rejected(self):
        self.output.unlink()
        with self.assertRaisesRegex(SystemExit, "does not exist"):
            self.invoke([self.artifact()])

    def test_directory_output_rejected(self):
        self.output.unlink()
        self.output.mkdir()
        with self.assertRaisesRegex(SystemExit, "does not exist"):
            self.invoke([self.artifact()])

    def test_nonzero_exit_rejected_even_with_existing_artifact(self):
        diagnostics = {"reason": "compiler-message", "message": {"rendered": "error: test fixture\n"}}
        with self.assertRaises(subprocess.CalledProcessError) as caught:
            self.invoke([diagnostics, self.artifact()], status=101)
        self.assertEqual(caught.exception.returncode, 101)
        self.assertIn("error: test fixture", self.stderr.getvalue())

    def test_diagnostics_forwarded_on_success(self):
        diagnostic = {"reason": "compiler-message", "message": {"rendered": "warning: test fixture\n"}}
        self.assertEqual(self.invoke([diagnostic, self.artifact()]), self.output)
        self.assertEqual(self.stderr.getvalue(), "warning: test fixture\n")

    def test_malformed_stdout_is_forwarded_and_rejected(self):
        with self.assertRaisesRegex(SystemExit, "exactly one"):
            self.invoke([], raw="not JSON\n" + json.dumps(self.artifact()) + "\n")
        self.assertIn("not JSON", self.stderr.getvalue())

    def test_malformed_messages_fail_closed_without_masking_cargo_exit(self):
        cases = [
            [],
            {"reason": "compiler-message", "message": []},
            {"reason": "compiler-message", "message": {"rendered": 123}},
            self.artifact(target=[]),
            self.artifact(profile=[]),
        ]
        for malformed in cases:
            with self.subTest(malformed=malformed), self.assertRaisesRegex(SystemExit, "exactly one"):
                self.invoke([malformed, self.artifact()])
            with self.subTest(status=101, malformed=malformed), self.assertRaises(subprocess.CalledProcessError):
                self.invoke([malformed, self.artifact()], status=101)

    def test_unsuccessful_build_finished_rejected_even_if_exit_zero(self):
        with self.assertRaisesRegex(SystemExit, "exactly one"):
            self.invoke([self.artifact(), {"reason": "build-finished", "success": False}], finished=False)

    def test_missing_build_finished_rejected(self):
        with self.assertRaisesRegex(SystemExit, "exactly one"):
            self.invoke([self.artifact()], finished=False)

    def test_duplicate_build_finished_rejected(self):
        with self.assertRaisesRegex(SystemExit, "exactly one"):
            self.invoke([self.artifact(), {"reason": "build-finished", "success": True}])

    def test_build_finished_success_must_be_boolean_true(self):
        for value in (None, 1, 0, "true", "false", {}, []):
            with self.subTest(value=value), self.assertRaisesRegex(SystemExit, "exactly one"):
                self.invoke([self.artifact(), {"reason": "build-finished", "success": value}], finished=False)


def portable_module():
    # Import real packaging helpers. Every compiler/process call is mocked by tests.
    with patch.dict(sys.path_importer_cache, {}):
        return load_file("portable_under_test", SCRIPTS / "portable.py")


class PortableSelectionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="portable scope tests ")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.native = self.root / "desktop"
        self.native.mkdir()
        protocol = self.native / "src/declarative/protocol.rs"
        protocol.parent.mkdir(parents=True)
        protocol.write_text("pub const PROTOCOL: u32 = 4;\n")
        self.portable = portable_module()
        self.portable.NATIVE = self.native
        self.portable.BUILD = self.native / "build"
        self.portable.PORTABLE = self.portable.BUILD / "Proteus"
        self.base = self.root / "ready base"
        self.seed(self.base)
        self.client = self.root / "desktop executable"
        self.client.write_text("new desktop")
        self.resolved = []
        self.built = []
        self.executed = []
        for name in ("resolve", "limit", "build_binary", "run", "install_file",
                     "install_component_notices", "assemble_modules", "source_record", "version"):
            patcher = patch.object(self.portable, name)
            setattr(self, name, patcher.start())
            self.addCleanup(patcher.stop)
        self.resolve.side_effect = self.fake_resolve
        self.build_binary.side_effect = self.fake_build
        self.run.side_effect = self.fake_run
        self.install_file.side_effect = self.copy_file
        self.version.return_value = "9.0.0"
        self.source_record.side_effect = lambda name, root: {"commit": "current-" + name}

    def seed(self, base):
        files = ("bin/proteus", "bin/proteus-native", "bin/old-module", "proteus-native",
                 "install-shortcut", "share/system_proxy.py", "share/icons/icon",
                 "share/configs/profile", "share/extensions/old/bin/ext")
        for name in files:
            path = base / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("old " + name)
            if name.startswith("bin/") or "/bin/" in name or name in ("proteus-native", "install-shortcut"):
                path.chmod(0o755)
        for name in self.portable.SOURCE_NAMES.values():
            path = base / "share/licenses" / name / "LICENSE"
            path.parent.mkdir(parents=True)
            path.write_text("old license " + name)
        self.write_json(base / "share/extensions/old/extension.json",
                        {"protocol": 4, "command": ["bin/ext"]})
        self.write_json(base / "share/modules/old/module.json", {"command": ["bin/old-module"]})
        self.write_json(base / "share/release-sources.json", {
            "version": "1.2.3", "sources": {name: {"commit": "old-" + name}
                                                 for name in self.portable.SOURCE_NAMES.values()},
        })

    @staticmethod
    def write_json(path, record):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(record))

    def copy_file(self, source, target, executable=False):
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes() if source.is_file() else b"new desktop asset")
        if executable:
            target.chmod(0o755)

    def fake_resolve(self, name):
        self.resolved.append(name)
        root = self.root / name
        if name == "extensions" and not (root / "packages/new/extension.json").exists():
            self.write_json(root / "packages/new/extension.json", {"protocol": 4})
        return root

    def fake_build(self, root, package, binary, flags, manifest):
        self.built.append((package, flags))
        return self.client

    def fake_run(self, command, cwd):
        self.executed.append((command, cwd))
        destination = Path(command[command.index("--bundle") + 1])
        if cwd.name == "extensions":
            self.write_json(destination / "new/extension.json", {"protocol": 4, "command": ["bin/ext"]})
            path = destination / "new/bin/ext"
        else:
            self.write_json(destination / "share/modules/new/module.json", {"command": ["bin/new-module"]})
            path = destination / "bin/new-module"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("new selected binary")
        path.chmod(0o755)

    def invoke(self, *args):
        with contextlib.redirect_stdout(io.StringIO()):
            self.portable.main(["--base", str(self.base), "--no-archive", *args])

    def record(self):
        return json.loads((self.portable.PORTABLE / "share/release-sources.json").read_text())

    def test_default_builds_only_desktop_and_preserves_whole_reused_components(self):
        self.invoke()
        self.assertEqual(self.built, [("proteus-native", ["--release"])])
        self.assertEqual(self.resolved, [])
        self.run.assert_not_called()
        self.assemble_modules.assert_not_called()
        for name in ("bin/proteus", "share/extensions/old/bin/ext", "share/extensions/old/extension.json",
                     "share/configs/profile", "share/licenses/Proteus-core/LICENSE", "share/modules/old/module.json"):
            self.assertEqual((self.portable.PORTABLE / name).read_bytes(), (self.base / name).read_bytes())
        record = self.record()
        self.assertEqual(record["sources"]["Proteus-core"]["commit"], "old-Proteus-core")
        self.assertEqual(record["sources"]["Proteus-desktop"]["commit"], "current-Proteus-desktop")
        self.assertEqual(record["build_components"], ["desktop"])
        self.assertEqual(record["build_mode"], "partial")
        self.assertIn("proteus", record["binary_sha256"])

    def test_extension_only_never_builds_desktop_or_resolves_core_and_replaces_assets(self):
        self.invoke("--components", "extensions")
        self.build_binary.assert_not_called()
        self.assertEqual(self.resolved, ["extensions"])
        self.assertEqual(len(self.executed), 1)
        self.assertNotIn("--no-build", self.executed[0][0])
        self.assertFalse((self.portable.PORTABLE / "share/extensions/old").exists())
        self.assertEqual((self.portable.PORTABLE / "bin/proteus-native").read_text(), "old bin/proteus-native")
        self.assertEqual(self.record()["version"], "1.2.3")

    def test_modules_only_copy_sdk_sources_without_core_cargo(self):
        self.invoke("--components", "modules")
        self.build_binary.assert_not_called()
        self.assertEqual(self.resolved, ["harness", "modules"])
        self.assemble_modules.assert_called_once()
        self.assertEqual(self.record()["sources"]["Proteus-core"]["commit"], "old-Proteus-core")
        self.assertEqual(self.record()["module_sdk_source"]["commit"], "current-Proteus-core")
        self.assertFalse((self.portable.PORTABLE / "bin/old-module").exists())
        self.assertFalse((self.portable.PORTABLE / "share/modules/old").exists())

    def seed_package_updates(self):
        path = self.base / "share/release-sources.json"
        record = json.loads(path.read_text())
        record["package_source_overrides"] = {
            group: {"old": {"source": {"commit": "old-package-" + group},
                            "sha256": "old-hash-" + group}}
            for group in ("modules", "extensions")
        }
        record["build_package_updates"] = {
            group: ["old"] for group in ("modules", "extensions")
        }
        record["local_extension_variant"] = {"enabled_additions": ["local-renderer"]}
        self.write_json(path, record)
        return record

    def test_whole_group_replacement_clears_only_selected_package_provenance(self):
        original = self.seed_package_updates()
        for selected, reused in (("modules", "extensions"), ("extensions", "modules")):
            with self.subTest(selected=selected):
                self.invoke("--components", selected)
                record = self.record()
                for key in ("package_source_overrides", "build_package_updates"):
                    self.assertEqual(record[key], {reused: original[key][reused]})
                self.assertEqual(record["local_extension_variant"], original["local_extension_variant"])
                self.assertEqual(record["sources"][self.portable.SOURCE_NAMES[reused]],
                                 original["sources"][self.portable.SOURCE_NAMES[reused]])
                self.assertEqual(json.loads((self.base / "share/release-sources.json").read_text()), original)

    def test_replacing_both_groups_removes_empty_package_provenance_containers(self):
        original = self.seed_package_updates()
        self.invoke("--components", "modules", "extensions")
        record = self.record()
        for key in ("package_source_overrides", "build_package_updates"):
            self.assertNotIn(key, record)
        self.assertEqual(record["local_extension_variant"], original["local_extension_variant"])

    def test_desktop_update_preserves_reused_package_provenance(self):
        original = self.seed_package_updates()
        self.invoke()
        record = self.record()
        for key in ("package_source_overrides", "build_package_updates", "local_extension_variant"):
            self.assertEqual(record[key], original[key])

    def test_reuse_preserves_package_provenance_bytes(self):
        self.seed_package_updates()
        self.invoke("--reuse-binaries")
        self.assertEqual((self.portable.PORTABLE / "share/release-sources.json").read_bytes(),
                         (self.base / "share/release-sources.json").read_bytes())

    def test_reuse_is_exact_copy_no_resolve_build_or_metadata_rewrite(self):
        self.invoke("--reuse-binaries")
        for mock in (self.build_binary, self.run, self.limit, self.resolve, self.source_record):
            mock.assert_not_called()
        for path in self.base.rglob("*"):
            if path.is_file():
                self.assertEqual(path.read_bytes(), (self.portable.PORTABLE / path.relative_to(self.base)).read_bytes())

    def test_plan_has_no_source_resolution_build_limits_or_base_requirement(self):
        with contextlib.redirect_stdout(io.StringIO()):
            self.portable.main(["--plan", "--base", str(self.root / "absent")])
        for mock in (self.build_binary, self.run, self.resolve, self.limit):
            mock.assert_not_called()
        self.assertFalse(self.portable.BUILD.exists())

    def test_missing_or_incomplete_base_fails_before_build_or_resolution(self):
        for missing in ("share/release-sources.json", "share/extensions/old/bin/ext", "bin/proteus"):
            path = self.base / missing
            data = path.read_bytes()
            path.unlink()
            with self.subTest(missing=missing), self.assertRaisesRegex(SystemExit, "--full"):
                self.invoke()
            path.write_bytes(data)
        self.build_binary.assert_not_called()
        self.resolve.assert_not_called()
        self.limit.assert_not_called()

    def test_protocol_mismatch_requires_explicit_extensions_before_cargo(self):
        self.write_json(self.base / "share/extensions/old/extension.json",
                        {"protocol": 3, "command": ["bin/ext"]})
        with self.assertRaisesRegex(SystemExit, "--components desktop extensions"):
            self.invoke()
        self.build_binary.assert_not_called()
        self.resolve.assert_not_called()

    def test_nonexecutable_ready_component_is_rejected_before_cargo(self):
        (self.base / "bin/proteus").chmod(0o644)
        with self.assertRaisesRegex(SystemExit, "not executable"):
            self.invoke()
        self.build_binary.assert_not_called()
        self.resolve.assert_not_called()

    def test_extension_only_checks_selected_protocol_independently_of_desktop_sources(self):
        self.write_json(self.root / "extensions/packages/new/extension.json", {"protocol": 5})
        with self.assertRaisesRegex(SystemExit, "--components desktop extensions"):
            self.invoke("--components", "extensions")
        self.build_binary.assert_not_called()
        self.run.assert_not_called()

    def test_selected_build_failure_keeps_previous_package(self):
        self.portable.PORTABLE.mkdir(parents=True)
        sentinel = self.portable.PORTABLE / "previous"
        sentinel.write_text("working")
        self.build_binary.side_effect = SystemExit("fake compiler failure")
        with self.assertRaisesRegex(SystemExit, "fake compiler failure"):
            self.invoke()
        self.assertEqual(sentinel.read_text(), "working")
        self.assertEqual(list(self.portable.BUILD.glob("Proteus.next-*")), [])

    def test_full_is_explicit_and_invokes_all_components_without_requiring_base(self):
        # Keep the real selector/installer paths, substitute metadata/config IO only.
        full_record = {"version": "9.0.0", "sources": {name: {"commit": "new"}
                       for name in self.portable.SOURCE_NAMES.values()}}
        def metadata(stage, *roots):
            self.write_json(stage / "share/release-sources.json", full_record)
        def notices(stage, name, root):
            path = stage / "share/licenses" / name
            path.mkdir(parents=True)
        self.install_component_notices.side_effect = notices
        def core(stage, root, flags):
            self.fake_build(root, "proteus-core", "proteus", flags, root / "Cargo.toml")
            self.copy_file(self.client, stage / "bin/proteus", executable=True)
            path = stage / "share/configs/profile"
            path.parent.mkdir(parents=True)
            path.write_text("fresh")
        with patch.object(self.portable, "install_core", side_effect=core), \
             patch.object(self.portable, "write_manifest", side_effect=metadata):
            self.invoke("--full")
        self.assertEqual([name for name, flags in self.built], ["proteus-native", "proteus-core"])
        self.assertEqual(self.resolved, ["harness", "modules", "extensions"])
        self.assertEqual(len(self.executed), 2)
        self.assertNotIn("build_mode", self.record())

    def test_invalid_combinations_and_removed_fast_are_parser_errors(self):
        for flags in (("--full", "--components", "desktop"), ("--full", "--reuse-binaries"),
                      ("--components", "desktop", "--reuse-binaries"),
                      ("--reuse-binaries", "--debug"), ("--fast",), ("--reuse-hosts",),
                      ("--components", "desktop", "desktop")):
            with self.subTest(flags=flags), contextlib.redirect_stderr(io.StringIO()), \
                 self.assertRaises(SystemExit) as caught:
                self.invoke(*flags)
            self.assertEqual(caught.exception.code, 2)
        self.build_binary.assert_not_called()

    def test_publication_failure_restores_previous_package(self):
        from portable_state import publish
        destination = self.root / "published"
        destination.mkdir()
        (destination / "working").write_text("old")
        stage = self.root / "stage"
        stage.mkdir()
        original = Path.rename
        def rename(path, target):
            if path == stage:
                raise OSError("fake publish failure")
            return original(path, target)
        with patch.object(Path, "rename", rename), self.assertRaisesRegex(OSError, "fake publish"):
            publish(stage, destination)
        self.assertEqual((destination / "working").read_text(), "old")
        self.assertFalse(destination.with_name("published.previous").exists())


class ExistingBinaryTests(unittest.TestCase):
    def test_reuse_target_is_relative_to_each_checkout_without_cargo(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for target in (Path("target"), Path("relative cache"), root / "absolute cache"):
                with self.subTest(target=target):
                    output = root / target / "fast/proteus-native"
                    output.parent.mkdir(parents=True, exist_ok=True)
                    output.write_bytes(b"existing")
                    with patch.dict(os.environ, {"CARGO_TARGET_DIR": str(target)}), \
                         patch.object(cargo_binary.subprocess, "run") as run:
                        self.assertEqual(cargo_binary.existing_binary(root, "fast", "proteus-native"), output)
                    run.assert_not_called()

    def test_missing_reused_output_fails_before_assembly(self):
        with tempfile.TemporaryDirectory() as directory, \
             patch.dict(os.environ, {"CARGO_TARGET_DIR": "missing"}), \
             patch.object(cargo_binary.subprocess, "run") as run:
            with self.assertRaisesRegex(SystemExit, "No existing binary"):
                cargo_binary.existing_binary(directory, "fast", "proteus-native")
            run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
