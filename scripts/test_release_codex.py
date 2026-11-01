"""Codex discovery uses only synthetic installed files and controlled environments."""
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import codex_executable as codex


class CodexExecutableTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="codex discovery ")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.env = {"HOME": str(self.root / "home"), "PATH": str(self.root / "path")}
        self.bundle = self.root / "app/resources/codex"
        self.addCleanup(patch.stopall)
        patch.object(codex, "LINUX_CODEX_PROGRAM", self.bundle).start()
        patch.object(codex.sys, "platform", "linux").start()

    def executable(self, path):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("#!/bin/sh\nexit 0\n")
        path.chmod(0o755)
        return str(path)

    def test_explicit_override_is_preserved_even_when_missing(self):
        self.executable(self.bundle)
        for value in ("/missing/with spaces/codex", "wrapper --literal", " "):
            self.env["PROTEUS_CODEX_EXECUTABLE"] = value
            with patch.object(codex.shutil, "which") as lookup:
                codex.configure_codex(self.env)
            lookup.assert_not_called()
            self.assertEqual(self.env["PROTEUS_CODEX_EXECUTABLE"], value)

    def test_path_wins_before_local_and_app_installs_without_changing_path(self):
        expected = self.executable(Path(self.env["PATH"]) / "codex")
        self.executable(Path(self.env["HOME"]) / ".npm-global/bin/codex")
        self.executable(self.bundle)
        before = dict(self.env)
        codex.configure_codex(self.env)
        self.assertEqual(self.env, {**before, "PROTEUS_CODEX_EXECUTABLE": expected})

    def test_common_local_installs_work_without_shell_path(self):
        for directory in (".local/bin", ".npm-global/bin", ".cargo/bin"):
            with self.subTest(directory=directory):
                path = Path(self.env["HOME"]) / directory / "codex"
                expected = self.executable(path)
                self.assertEqual(codex.codex_program(self.env), expected)
                path.unlink()

    def test_configured_npm_prefix_precedes_default_locations(self):
        expected = self.executable(self.root / "custom npm/bin/codex")
        self.executable(Path(self.env["HOME"]) / ".npm-global/bin/codex")
        for key in ("npm_config_prefix", "NPM_CONFIG_PREFIX"):
            self.assertEqual(codex.codex_program({**self.env, key: str(self.root / "custom npm")}), expected)

    def test_linux_app_fallback_skips_non_executable_and_directory_candidates(self):
        local = Path(self.env["HOME"]) / ".local/bin/codex"
        self.executable(local)
        local.chmod(0o644)
        (Path(self.env["HOME"]) / ".npm-global/bin/codex").mkdir(parents=True)
        expected = self.executable(self.bundle)
        self.assertEqual(codex.codex_program(self.env), expected)
        with patch.object(codex.sys, "platform", "darwin"):
            self.assertIsNone(codex.codex_program(self.env))

    def test_missing_install_leaves_environment_unchanged(self):
        for extra in ({}, {"PROTEUS_CODEX_EXECUTABLE": ""}):
            environment = {**self.env, **extra}
            before = dict(environment)
            codex.configure_codex(environment)
            self.assertEqual(environment, before)


if __name__ == "__main__":
    unittest.main()
