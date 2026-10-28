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
    stubs = {}
    for name, exports in {
        "sources": ("resolve",),
        "limits": ("limit",),
        "portable_modules": ("assemble",),
        "release_metadata": ("install_notices", "version", "write_manifest"),
    }.items():
        module = types.ModuleType(name)
        for export in exports:
            setattr(module, export, Mock())
        stubs[name] = module
    stubs["cargo_binary"] = cargo_binary
    with patch.dict(sys.modules, stubs):
        return load_file("portable_under_test", SCRIPTS / "portable.py")


class PortableWiringTests(unittest.TestCase):
    def setUp(self):
        self.portable = portable_module()
        self.portable.ROOT = Path("/synthetic/core")
        self.portable.MODULES = Path("/synthetic/modules")
        self.portable.EXTENSIONS = Path("/synthetic/extensions")
        self.sources = {
            "harness": self.portable.ROOT, "modules": self.portable.MODULES,
            "extensions": self.portable.EXTENSIONS,
        }

    def invoke_main(self, flags):
        outputs = [Path("/cache/core"), Path("/cache/module"), Path("/cache/native")]
        with patch.object(self.portable, "resolve", side_effect=self.sources.__getitem__), \
             patch.object(self.portable, "build_binary", side_effect=outputs) as build, \
             patch.object(self.portable, "assemble") as assemble, \
             patch.object(self.portable, "archive") as archive, \
             patch.object(sys, "argv", ["portable.py", *flags]), \
             contextlib.redirect_stdout(io.StringIO()):
            self.portable.main()
        return build, assemble, archive, outputs

    def test_release_and_no_archive_keep_release_builds(self):
        for flags in ([], ["--no-archive"]):
            with self.subTest(flags=flags):
                build, assemble, archive, outputs = self.invoke_main(flags)
                self.assertEqual([call.args[3] for call in build.call_args_list],
                                 [["--release"], ["--release"], ["--release"]])
                self.assertEqual([call.args[1:3] for call in build.call_args_list], [
                    ("proteus-core", "proteus"),
                    ("proteus-reference-module", "proteus-reference-module"),
                    ("proteus-native", "proteus-native"),
                ])
                self.assertEqual(build.call_args_list[0].args[4],
                                 self.portable.ROOT / "crates/proteus-core/Cargo.toml")
                self.assertEqual(build.call_args_list[1].args[4],
                                 self.portable.MODULES / "components/process-module/Cargo.toml")
                self.assertEqual(build.call_args_list[2].args[4],
                                 self.portable.NATIVE / "Cargo.toml")
                assemble.assert_called_once_with(*outputs)
                self.assertEqual(archive.call_count, 0 if flags else 1)

    def test_fast_and_debug_existing_semantics_unchanged(self):
        for flag, expected in [
            ("--fast", [["--release"], ["--release"], ["--profile", "fast"]]),
            ("--debug", [[], [], []]),
        ]:
            with self.subTest(flag=flag):
                build, _, archive, _ = self.invoke_main([flag])
                self.assertEqual([call.args[3] for call in build.call_args_list], expected)
                self.assertEqual(archive.call_count, 0 if flag == "--fast" else 1)

    def test_build_failure_prevents_any_assembly_or_archive(self):
        with patch.object(self.portable, "resolve", side_effect=self.sources.__getitem__), \
             patch.object(self.portable, "build_binary", side_effect=SystemExit("missing artifact")), \
             patch.object(self.portable, "assemble") as assemble, \
             patch.object(self.portable, "archive") as archive, \
             patch.object(sys, "argv", ["portable.py"]):
            with self.assertRaises(SystemExit):
                self.portable.main()
        assemble.assert_not_called()
        archive.assert_not_called()

    def test_reuse_skips_all_builds_and_limits_and_keeps_fast_profile(self):
        outputs = [Path("/cache/core"), Path("/cache/module"), Path("/cache/native")]
        with patch.object(self.portable, "resolve", side_effect=self.sources.__getitem__), \
             patch.object(self.portable, "build_binary") as build, \
             patch.object(self.portable, "existing_binary", side_effect=outputs) as existing, \
             patch.object(self.portable, "limit") as limit, \
             patch.object(self.portable, "assemble") as assemble, \
             patch.object(self.portable, "archive") as archive, \
             patch.object(sys, "argv", ["portable.py", "--fast", "--reuse-binaries"]), \
             contextlib.redirect_stdout(io.StringIO()):
            self.portable.main()
        build.assert_not_called()
        limit.assert_not_called()
        archive.assert_not_called()
        self.assertEqual([call.args[1:] for call in existing.call_args_list], [
            ("release", "proteus"), ("release", "proteus-reference-module"),
            ("fast", "proteus-native"),
        ])
        assemble.assert_called_once_with(*outputs, reuse_binaries=True)

    def test_assemble_uses_exact_binaries_and_extensions_source_cwd(self):
        agent, module, client = [Path("/external cache") / name for name in ("core", "module", "client")]
        with patch.object(self.portable, "install_file") as install, \
             patch.object(self.portable, "tracked_configs", return_value=[]), \
             patch.object(self.portable, "run") as run, \
             patch.object(self.portable.shutil, "rmtree"), \
             patch.object(Path, "rename"):
            self.portable.assemble(agent, module, client)
        staging = self.portable.BUILD / "Proteus.next"
        self.assertEqual(install.call_args_list[0].args, (client, staging / "bin/proteus-native", True))
        self.assertEqual(install.call_args_list[1].args, (agent, staging / "bin/proteus", True))
        self.assertEqual(install.call_args_list[2].args, (module, staging / "bin/proteus-reference-module", True))
        run.assert_called_once_with([
            sys.executable, str(self.portable.EXTENSIONS / "package.py"),
            "--bundle", str(staging / "share/extensions"),
        ], self.portable.EXTENSIONS)
        # package.py builds at its own source ROOT and interprets the inherited
        # relative CARGO_TARGET_DIR at its process cwd. These now coincide.
        relative = Path("relative cache")
        child_cwd = run.call_args.args[1]
        self.assertEqual(child_cwd / relative, self.portable.EXTENSIONS / relative)
        self.assertTrue(Path(run.call_args.args[0][1]).is_absolute())
        self.assertTrue(Path(run.call_args.args[0][3]).is_absolute())


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
