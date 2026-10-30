"""Synthetic repositories prove release guards and exact-source export behavior."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import release
from release_metadata import VERSION_PATTERN, install_notices


class ReleaseSourcesTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="proteus-release-test-")
        self.addCleanup(self.temporary.cleanup)
        base = Path(self.temporary.name)
        self.desktop = base / "desktop"
        self.source = {name: base / name for name in ("harness", "modules", "extensions")}
        for path in (self.desktop, *self.source.values()):
            path.mkdir()
            (path / ".gitignore").write_text("build/\nignored-secret.txt\n")
            (path / "Cargo.toml").write_text('[package]\nname="fixture"\nversion="0.1.0-alpha.2"\n')
            (path / "LICENSE").write_text("synthetic MIT notice\n")
            subprocess.run(["git", "init", "-q", path], check=True)
        for root, relative in ((self.source["harness"], "crates/proteus-core"),
                               (self.source["modules"], "components/process-module")):
            package = root / relative
            package.mkdir(parents=True)
            (package / "Cargo.toml").write_text('[package]\nname="fixture"\nversion="0.1.0-alpha.2"\n')
        self.specs = {}
        self.env = {"GITHUB_REF_TYPE": "branch", "PROTEUS_RELEASE_TAG": ""}
        for name, path in {"desktop": self.desktop, **self.source}.items():
            self.commit(path)
            (path / "ignored-secret.txt").write_text("must never enter source archive")
            if name != "desktop":
                key = "PROTEUS_TEST_SOURCE_" + name.upper()
                self.env[key] = str(path)
                self.specs[name] = {"env": key, "marker": "Cargo.toml", "rev": release.git(path, "rev-parse", "HEAD")}
        for target, value in (("ROOT", self.desktop), ("BUILD", self.desktop / "build"), ("SPECS", self.specs)):
            patcher = patch.object(release, target, value)
            patcher.start()
            self.addCleanup(patcher.stop)
        patcher = patch.dict(os.environ, self.env)
        patcher.start()
        self.addCleanup(patcher.stop)

    def commit(self, root):
        subprocess.run(["git", "add", "."], cwd=root, check=True)
        subprocess.run(["git", "-c", "user.name=Release Test", "-c", "user.email=test@example.invalid",
                        "commit", "-qm", "synthetic fixture"], cwd=root, check=True)

    def test_clean_exact_sources_and_matching_tag_are_accepted(self):
        self.assertEqual(release.check_sources("v0.1.0-alpha.2"), self.source)

    def test_changed_commit_is_rejected_even_when_checkout_is_clean(self):
        root = self.source["modules"]
        (root / "new.txt").write_text("new revision")
        self.commit(root)
        with self.assertRaisesRegex(SystemExit, "does not match"):
            release.check_sources()

    def test_untracked_files_and_modified_sources_are_rejected(self):
        path = self.source["harness"] / "unexpected.txt"
        path.write_text("uncommitted")
        with self.assertRaisesRegex(SystemExit, "dirty"):
            release.check_sources()
        path.unlink()
        (self.source["harness"] / "LICENSE").write_text("modified")
        with self.assertRaisesRegex(SystemExit, "dirty"):
            release.check_sources()

    def test_mismatched_package_version_and_tag_are_rejected(self):
        with self.assertRaisesRegex(SystemExit, "Release tag must be"):
            release.check_sources("v0.1.0-alpha.3")
        root = self.source["modules"]
        package = root / "components/process-module/Cargo.toml"
        package.write_text(package.read_text().replace("alpha.2", "alpha.3"))
        self.commit(root)
        self.specs["modules"]["rev"] = release.git(root, "rev-parse", "HEAD")
        with self.assertRaisesRegex(SystemExit, "package version"):
            release.check_sources()

    def test_tag_build_cannot_skip_tag_validation(self):
        with patch.dict(os.environ, {"GITHUB_REF_TYPE": "tag"}):
            with self.assertRaisesRegex(SystemExit, "explicitly validate"):
                release.check_sources()

    def test_source_archive_uses_commits_and_excludes_ignored_local_data(self):
        target = self.desktop / "build/sources.tar.gz"
        target.parent.mkdir()
        release.source_archive(self.source, target)
        with tarfile.open(target) as archive:
            names = archive.getnames()
            for name in ("Proteus-core", "Proteus-modules", "Proteus-desktop", "Proteus-desktop-extensions"):
                self.assertIn(f"sources/{name}/LICENSE", names)
            self.assertFalse(any("ignored-secret" in name or "/.git/" in name for name in names))

    def test_stale_archive_cannot_reuse_smoke_evidence(self):
        build = self.desktop / "build"
        (build / "Proteus/share/licenses").mkdir(parents=True)
        record = {"sources": {"fixture": {"dirty": False}}, "version": "0.1.0-alpha.2"}
        (build / "Proteus/share/release-sources.json").write_text(json.dumps(record))
        (build / "Proteus/share/licenses/README.txt").write_text("licenses")
        archive = build / f"Proteus-0.1.0-alpha.2-linux-{os.uname().machine}.tar.gz"
        archive.write_bytes(b"new archive")
        (build / "verification").mkdir()
        (build / "verification/release-smoke.json").write_text(json.dumps({
            "status": "passed", "source_manifest": record, "archive_sha256": "old-archive",
        }))
        with patch.object(release, "manifest", return_value=record):
            with self.assertRaisesRegex(SystemExit, "differs from"):
                release.assets()


class MetadataTests(unittest.TestCase):
    def test_sdk_gate_builds_locked_starter_and_invokes_real_conformance(self):
        with tempfile.TemporaryDirectory() as temporary:
            core = Path(temporary)
            for configured, expected in (("relative-target", core / "relative-target"),
                                         (str(core / "absolute-target"), core / "absolute-target")):
                with patch.dict(os.environ, {"CARGO_TARGET_DIR": configured}), patch.object(release, "run") as run:
                    release.sdk_gate(core)
                calls = run.call_args_list
                self.assertEqual(len(calls), 3)
                self.assertIn("--locked", calls[0].args[0])
                self.assertIn("examples/modules/rust-tool/Cargo.toml", calls[0].args[0])
                self.assertIn("--locked", calls[1].args[0])
                self.assertEqual(calls[0].kwargs["env"]["CARGO_TARGET_DIR"], str(expected))
                self.assertIn(expected / "debug/proteus-component-conformance", calls[2].args[0])
                self.assertIn(expected / "debug/proteus-rust-tool-example", calls[2].args[0])

    def test_release_version_rejects_malformed_or_path_like_values(self):
        for value in ("0.1.0-alpha.2", "0.0.0-dev", "1.2.3", "1.0.0-rc-1"):
            self.assertIsNotNone(re.fullmatch(VERSION_PATTERN, value))
        for value in ("../0.1.0", "01.0.0", "0.1", "0.1.0-alpha.02", "0.1.0/escape", "v0.1.0"):
            self.assertIsNone(re.fullmatch(VERSION_PATTERN, value))

    def test_packaged_notices_retain_each_repository_and_vendored_notice(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            roots = [base / name for name in ("desktop", "core", "modules", "extensions")]
            for root in roots:
                root.mkdir()
                (root / "LICENSE").write_text(root.name)
            for relative in ("tool/codex-patch", "hook/codex-context-preparation/src/upstream"):
                path = roots[2] / relative
                path.mkdir(parents=True)
                for name in ("LICENSE", "NOTICE"):
                    (path / name).write_text(relative + name)
            for vendored_name in ("gpui-fast-wgpu", "gpui-component"):
                vendored = roots[0] / "vendor" / vendored_name
                vendored.mkdir(parents=True)
                for name in ("LICENSE.upstream", "README.proteus.md"):
                    (vendored / name).write_text("synthetic " + vendored_name + " " + name)
            font = roots[0] / "assets/fonts/noto-color-emoji"
            font.mkdir(parents=True)
            for name in ("OFL.txt", "UPSTREAM.md"):
                (font / name).write_text("synthetic Noto " + name)
            portable = base / "portable"
            install_notices(portable, *roots)
            self.assertEqual((portable / "share/licenses/Proteus-core/LICENSE").read_text(), "core")
            self.assertTrue((portable / "share/licenses/Proteus-modules/tool/codex-patch/NOTICE").is_file())
            self.assertEqual(
                (portable / "share/licenses/Proteus-desktop/vendor/gpui-fast-wgpu/LICENSE.upstream").read_text(),
                "synthetic gpui-fast-wgpu LICENSE.upstream",
            )
            self.assertEqual(
                (portable / "share/licenses/Proteus-desktop/vendor/gpui-component/LICENSE.upstream").read_text(),
                "synthetic gpui-component LICENSE.upstream",
            )
            self.assertIn("Apache-2.0", (portable / "share/licenses/README.txt").read_text())
            for name in ("OFL.txt", "UPSTREAM.md"):
                self.assertEqual(
                    (portable / "share/licenses/Proteus-desktop/assets/fonts/noto-color-emoji" / name).read_text(),
                    "synthetic Noto " + name,
                )


class GateScopeTests(unittest.TestCase):
    def test_release_desktop_gate_explicitly_prepares_extensions(self):
        sources = {name: Path('/synthetic') / name for name in ('harness', 'modules', 'extensions')}
        with patch.object(release, 'check_sources', return_value=sources), patch.object(release, 'run') as run:
            release.gates('desktop')
        calls = [list(call.args[0]) for call in run.call_args_list]
        self.assertIn([sys.executable, 'scripts/test.py', '--build-extensions'], calls)
        self.assertFalse(any('cargo' == str(arg) and 'build' in call for call in calls for arg in call))

    def test_partial_portable_is_rejected_by_release_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            build = Path(directory)
            portable = build / 'Proteus/share'
            portable.mkdir(parents=True)
            expected = {'sources': {'Desktop': {'dirty': False}}}
            (portable / 'release-sources.json').write_text(json.dumps({**expected, 'build_mode': 'partial'}))
            sources = {name: Path('/synthetic') / name for name in ('harness', 'modules', 'extensions')}
            with patch.object(release, 'BUILD', build), patch.object(release, 'check_sources', return_value=sources), \
                 patch.object(release, 'manifest', return_value=expected):
                with self.assertRaisesRegex(SystemExit, 'provenance'):
                    release.assets()


if __name__ == "__main__":
    unittest.main()
