"""Feature selection and environment plumbing without downloads or Cargo."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import cargo_binary
import desktop_toolchain
import portable


class DesktopToolchainTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="desktop toolchain ")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / "Cargo.toml").write_text('''
[package]
name = "proteus-native"
[features]
default = ["terminal"]
terminal = ["ghostty-vt"]
ghostty-vt = ["proteus-terminal/ghostty-vt"]
other = []
''')

    def test_effective_features_follow_defaults_aliases_and_cargo_boundary(self):
        for flags in ([], ["--features", "other"], ["--all-features"],
                      ["--no-default-features", "--features=ghostty-vt"],
                      ["--no-default-features", "--features", "other,terminal"],
                      ["--no-default-features", "-Fghostty-vt"],
                      ["--no-default-features", "-F", "proteus-native/ghostty-vt"],
                      ["--no-default-features", "--features", "proteus-terminal/ghostty-vt"],
                      ["--", "--no-default-features"]):
            with self.subTest(flags=flags):
                self.assertTrue(desktop_toolchain.needs_ghostty(self.root, flags))
        for flags in (["--no-default-features"],
                      ["--no-default-features", "--features=other"],
                      ["--no-default-features", "--", "--features=ghostty-vt"]):
            with self.subTest(flags=flags):
                self.assertFalse(desktop_toolchain.needs_ghostty(self.root, flags))

    def test_disabled_engine_never_prepares_or_changes_supplied_environment(self):
        original = {"ZIG": "/custom/zig", "CARGO_TARGET_DIR": "/cache with spaces"}
        with patch.object(desktop_toolchain.terminal_toolchain, "prepare") as prepare:
            result = desktop_toolchain.prepare_desktop(self.root, ["--no-default-features"], original)
        prepare.assert_not_called()
        self.assertEqual(result, original)
        self.assertIsNot(result, original)

    def test_enabled_engine_preserves_custom_zig_and_inherited_build_configuration(self):
        original = {"ZIG": "/custom/zig", "CARGO_TARGET_DIR": "/cache with spaces"}
        with patch.object(desktop_toolchain.terminal_toolchain, "prepare",
                          side_effect=lambda root, env: dict(env)) as prepare:
            result = desktop_toolchain.prepare_desktop(self.root, environ=original)
        prepare.assert_called_once_with(self.root, original)
        self.assertEqual(result, original)
        self.assertIsNot(prepare.call_args.args[1], original)

    def test_manifest_without_ghostty_default_never_downloads_for_default_build(self):
        (self.root / "Cargo.toml").write_text('[features]\nghostty-vt = []\n')
        with patch.object(desktop_toolchain.terminal_toolchain, "prepare") as prepare:
            desktop_toolchain.prepare_desktop(self.root)
        prepare.assert_not_called()

    def test_core_install_does_not_prepare_desktop_toolchain(self):
        output = self.root / "proteus"
        with patch.object(portable, "build_binary", return_value=output) as build, \
             patch.object(portable, "prepare_desktop") as prepare, \
             patch.object(portable, "install_file"), \
             patch.object(portable.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, b"")):
            portable.install_core(self.root / "stage", self.root / "core", ["--release"])
        prepare.assert_not_called()
        self.assertNotIn("environ", build.call_args.kwargs)

    def test_cargo_receives_prepared_environment_without_mutating_parent(self):
        output = self.root / "native"
        output.write_bytes(b"synthetic binary")
        stdout = "\n".join(map(json.dumps, [
            {"reason": "compiler-artifact", "manifest_path": str(self.root / "Cargo.toml"),
             "target": {"name": "proteus-native", "kind": ["bin"]}, "profile": {"test": False},
             "executable": str(output)},
            {"reason": "build-finished", "success": True},
        ]))
        environment = {"ZIG": "/custom/zig", "CARGO_TARGET_DIR": "/cache with spaces"}
        previous = dict(os.environ)
        with patch.object(cargo_binary.subprocess, "run",
                          return_value=subprocess.CompletedProcess([], 0, stdout)) as run, \
             contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(cargo_binary.build_binary(self.root, "proteus-native", "proteus-native",
                             [], self.root / "Cargo.toml", environ=environment), output)
        self.assertEqual(run.call_args.kwargs["env"], environment)
        self.assertEqual(dict(os.environ), previous)


if __name__ == "__main__":
    unittest.main()
