"""Ready artifact checks and development runner never compile external components."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

HERE = Path(__file__).parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runtime = load("runtime_paths")


def binary(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("#!/bin/sh\nexit 0\n")
    path.chmod(0o755)
    return path


def package(base, directory="share/extensions", manifest="extension.json", program="bin/sample"):
    folder = base / directory / "sample"
    folder.mkdir(parents=True)
    (folder / manifest).write_text(json.dumps({"command": [program]}))
    return folder


class ReadyTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)

    def test_missing_extensions_fail_with_actionable_hint(self):
        with self.assertRaisesRegex(SystemExit, "explicit build"):
            runtime.require_extensions(self.root, hint="explicit build")

    def test_manifest_commands_must_exist_and_be_executable(self):
        folder = package(self.root)
        for mode in (None, 0o644, 0o755):
            if mode is not None:
                binary(folder / "bin/sample").chmod(mode)
            if mode == 0o755:
                runtime.require_extensions(folder.parent, hint="build explicitly")
            else:
                with self.assertRaisesRegex(SystemExit, "sample.*missing executable"):
                    runtime.require_extensions(folder.parent, hint="build explicitly")

    def test_modules_use_runtime_root_relative_paths(self):
        package(self.root, "share/modules", "module.json", "bin/module")
        with self.assertRaisesRegex(SystemExit, "Module sample.*missing executable"):
            runtime.require_modules(self.root)
        binary(self.root / "bin/module")
        runtime.require_modules(self.root)

    def test_incompatible_extension_protocol_fails_before_a_desktop_build(self):
        native = self.root / "native"
        source = native / "src/declarative/protocol.rs"
        source.parent.mkdir(parents=True)
        source.write_text("pub const PROTOCOL: u32 = 5;")
        with self.assertRaisesRegex(SystemExit, "protocol differs"):
            runtime.require_extensions(package(self.root).parent, hint="explicit setup", native=native)


class RunTests(unittest.TestCase):
    def setUp(self):
        self.sources = types.ModuleType("sources")
        self.sources.ROOT = Path("/synthetic/desktop")
        self.sources.resolve = Mock(return_value=Path("/synthetic/extensions"))
        self.runtime = types.ModuleType("runtime_paths")
        self.runtime.executable = Mock(return_value=Path("/ready/proteus"))
        self.runtime.require_extensions = Mock()
        self.runtime.require_modules = Mock()
        with patch.dict(sys.modules, {"sources": self.sources, "runtime_paths": self.runtime}):
            self.runner = load("run")
        self.run = patch.object(self.runner.subprocess, "run").start()
        self.call = patch.object(self.runner.subprocess, "call", return_value=0).start()
        self.addCleanup(patch.stopall)
        patch.dict(os.environ, {"PATH": "/synthetic/bin"}, clear=True).start()
        # Use a managed redirect so unittest's following tests keep their stderr.
        self.redirect = contextlib.redirect_stderr(io.StringIO())
        self.redirect.__enter__()
        self.addCleanup(self.redirect.__exit__, None, None, None)

    def test_import_has_no_execution(self):
        self.sources.resolve.assert_not_called()
        self.runtime.require_extensions.assert_not_called()
        self.run.assert_not_called()
        self.call.assert_not_called()

    def test_default_builds_desktop_only_with_ready_runtime(self):
        self.assertEqual(self.runner.main(["/workspace", "--config", "profile"]), 0)
        self.run.assert_not_called()
        self.sources.resolve.assert_not_called()
        self.runtime.require_modules.assert_called_once_with(Path("/synthetic/desktop/build/Proteus"))
        call = self.call.call_args
        self.assertEqual(call.args[0], ["cargo", "run", "--locked", "-p", "proteus-native", "--", "/workspace", "--config", "profile"])
        self.assertEqual(call.kwargs["env"]["PROTEUS_BIN"], "/ready/proteus")
        self.assertEqual(call.kwargs["env"]["PROTEUS_EXTENSIONS_DIR"], "/synthetic/desktop/build/Proteus/share/extensions")
        self.assertTrue(call.kwargs["env"]["PATH"].startswith("/synthetic/desktop/build/Proteus/bin:"))

    def test_origin_does_not_require_local_backend_or_modules(self):
        self.runner.main(["--base", "/ready", "--", "--origin", "http://localhost:1", "--token", "fixture"])
        self.runtime.executable.assert_not_called()
        self.runtime.require_modules.assert_not_called()
        self.runtime.require_extensions.assert_called_once()
        self.assertEqual(self.call.call_args.args[0][-4:], ["--origin", "http://localhost:1", "--token", "fixture"])

    def test_overrides_and_explicit_extension_build(self):
        os.environ.update(PROTEUS_BIN="/custom/proteus", PROTEUS_EXTENSIONS_DIR="/custom/extensions")
        self.runner.main(["--base", "/ready"])
        self.runtime.executable.assert_called_once_with("/custom/proteus", path="/ready/bin:/synthetic/bin")
        self.assertEqual(self.runtime.require_extensions.call_args.args[0], Path("/custom/extensions"))
        self.runner.main(["--build-extensions", "--base", "/ready", "/workspace"])
        self.sources.resolve.assert_called_once_with("extensions")
        self.run.assert_called_once()
        self.assertEqual(self.call.call_args.kwargs["env"]["PROTEUS_EXTENSIONS_DIR"], "/synthetic/extensions/packages")

    def test_missing_ready_runtime_never_runs_cargo(self):
        self.runtime.executable.return_value = None
        with self.assertRaisesRegex(SystemExit, "ready backend"):
            self.runner.main([])
        self.call.assert_not_called()
        self.run.assert_not_called()
        self.runtime.require_extensions.assert_not_called()

    def test_missing_ready_extensions_never_runs_cargo(self):
        self.runtime.require_extensions.side_effect = SystemExit("missing extension executable")
        with self.assertRaisesRegex(SystemExit, "missing extension"):
            self.runner.main([])
        self.call.assert_not_called()
        self.run.assert_not_called()

    def test_wrapper_flags_after_delimiter_are_application_arguments(self):
        self.runner.main(["--", "--build-extensions", "--base", "/other"])
        self.sources.resolve.assert_not_called()
        self.assertEqual(self.call.call_args.args[0][-3:], ["--build-extensions", "--base", "/other"])


if __name__ == "__main__":
    unittest.main()
