"""Final portable compatibility gate with inert files and synthetic builders."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch


HERE = Path(__file__).resolve().parent
FEATURE = "documents.active_panes.v1"


def forbidden(*args, **kwargs):
    raise AssertionError("Unexpected build, source or command execution")


def module(name, **attributes):
    value = types.ModuleType(name)
    value.__dict__.update(attributes)
    return value


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, HERE / filename)
    value = importlib.util.module_from_spec(spec)
    with patch.dict(sys.modules, {name: value}):
        spec.loader.exec_module(value)
    return value


def load_portable():
    state = load("_compatibility_state", "portable_state.py")
    replacements = {
        "sources": module("sources", resolve=forbidden),
        "limits": module("limits", limit=forbidden),
        "cargo_binary": module("cargo_binary", build_binary=forbidden),
        "desktop_toolchain": module("desktop_toolchain", prepare_desktop=forbidden),
        "portable_modules": module("portable_modules", assemble=forbidden),
        "portable_state": state,
        "release_metadata": module("release_metadata", NOTICE_README="synthetic notices",
            install_component_notices=forbidden, source_record=forbidden,
            version=forbidden, write_manifest=forbidden),
    }
    with patch.dict(sys.modules, replacements):
        portable = load("_compatibility_portable", "portable.py")
    return portable, state, replacements


def put(path, text="inert fixture", executable=False):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    if executable:
        path.chmod(0o755)


def desktop_files(stage):
    for name in ("bin/proteus-native", "proteus-native", "install-shortcut"):
        put(stage / name, executable=True)
    put(stage / "share/system_proxy.py")
    put(stage / "share/icons/icon.png")


def core_files(stage):
    put(stage / "bin/proteus", executable=True)
    put(stage / "share/configs/default.config.toml")


def module_files(stage):
    put(stage / "bin/module", executable=True)
    put(stage / "share/modules/sample/module.json", json.dumps({"command": ["bin/module"]}))


def extension_files(folder, protocol=4, required=()):
    put(folder / "sample/bin/extension", executable=True)
    put(folder / "sample/extension.json", json.dumps({
        "command": ["bin/extension"], "protocol": protocol, "required_features": list(required),
    }))


def record(state, protocol=4, features=()):
    return {"version": "fixture", "desktop_protocol": protocol, "desktop_features": list(features),
            "sources": {name: {"commit": "synthetic"} for name in state.SOURCE_NAMES.values()}}


def seed(stage, state, protocol=4, features=(), extension_protocol=4, required=()):
    desktop_files(stage)
    core_files(stage)
    module_files(stage)
    extension_files(stage / "share/extensions", extension_protocol, required)
    for name in state.SOURCE_NAMES.values():
        put(stage / "share/licenses" / name / "LICENSE")
    put(stage / "share/release-sources.json", json.dumps(record(state, protocol, features)))


def snapshot(root):
    return {str(p.relative_to(root)): (p.read_bytes(), p.stat().st_mode)
            for p in root.rglob("*") if p.is_file()}


class PortableCompatibilityTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="portable-compatibility-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.portable, self.state, self.replacements = load_portable()
        p = self.portable
        p.NATIVE = self.root / "desktop"
        p.BUILD = self.root / "build"
        p.PORTABLE = p.BUILD / "Proteus"
        self.source = {name: self.root / name for name in ("harness", "modules", "extensions")}
        self.protocol = 4
        self.features = []
        self.shipped_protocol = 4
        self.shipped_required = []
        self.set_native()
        extension_files(self.source["extensions"] / "packages")
        seed(p.PORTABLE, self.state)
        p.resolve = Mock(side_effect=self.source.__getitem__)
        p.limit = Mock()
        p.install_desktop = Mock(side_effect=lambda stage, flags: desktop_files(stage))
        p.install_core = Mock(side_effect=lambda stage, core, flags: core_files(stage))
        p.assemble_modules = Mock()
        p.install_component_notices = lambda stage, name, root: put(stage / "share/licenses" / name / "LICENSE")
        p.source_record = lambda name, root: {"commit": "synthetic-new"}
        p.version = lambda root: "fixture-new"
        p.write_manifest = lambda stage, *roots: put(stage / "share/release-sources.json",
                                                    json.dumps(record(self.state, self.protocol, self.features)))

        def package(command, cwd):
            target = Path(command[command.index("--bundle") + 1])
            if cwd == self.source["modules"]:
                module_files(target)
            elif cwd == self.source["extensions"]:
                extension_files(target, self.shipped_protocol, self.shipped_required)
            else:
                forbidden()
        p.run = Mock(side_effect=package)
        p.publish = Mock(wraps=p.publish)
        p.archive = Mock(wraps=p.archive)

    def set_native(self):
        put(self.portable.NATIVE / "src/declarative/protocol.rs",
            f"pub const PROTOCOL: u32 = {self.protocol};\n"
            f"pub const FEATURES: &[&str] = &{json.dumps(self.features)};\n")

    def run_main(self, args):
        # The full branch imports NOTICE_README during actual assemble.
        with patch.dict(sys.modules, {"release_metadata": self.replacements["release_metadata"]}), \
             patch("subprocess.run", side_effect=forbidden), \
             patch("subprocess.check_output", side_effect=forbidden), \
             contextlib.redirect_stdout(io.StringIO()):
            self.portable.main([*args, "--no-archive"])

    def assert_rejected_unchanged(self, args, message):
        before = snapshot(self.portable.PORTABLE)
        with self.assertRaisesRegex(SystemExit, message):
            self.run_main(args)
        self.assertEqual(snapshot(self.portable.PORTABLE), before)
        self.portable.publish.assert_not_called()
        self.portable.archive.assert_not_called()
        self.assertEqual(list(self.portable.BUILD.glob("Proteus.next-*")), [])
        self.assertFalse(self.portable.PORTABLE.with_suffix(".previous").exists())

    def test_joint_protocol_mismatch_preserves_destination(self):
        self.shipped_protocol = 5
        self.assert_rejected_unchanged(["--components", "desktop", "extensions"], "protocol")

    def test_full_protocol_mismatch_preserves_destination(self):
        self.shipped_protocol = 5
        self.assert_rejected_unchanged(["--full"], "protocol")

    def test_joint_missing_feature_preserves_destination(self):
        self.shipped_required = [FEATURE]
        self.assert_rejected_unchanged(["--components", "desktop", "extensions"], "unsupported.*" + FEATURE)

    def test_full_missing_feature_preserves_destination(self):
        self.shipped_required = [FEATURE]
        self.assert_rejected_unchanged(["--full"], "unsupported.*" + FEATURE)

    def test_compatible_full_pair_publishes(self):
        self.features = [FEATURE]
        self.shipped_required = [FEATURE]
        self.set_native()
        self.run_main(["--full"])
        self.portable.publish.assert_called_once()
        saved = self.state.read_record(self.portable.PORTABLE)
        self.assertEqual(saved["desktop_features"], [FEATURE])
        self.assertEqual(self.state.extension_protocols(self.portable.PORTABLE), {4})

    def test_joint_update_repairs_incompatible_base(self):
        extension_files(self.portable.PORTABLE / "share/extensions", 5, [FEATURE])
        self.run_main(["--components", "desktop", "extensions"])
        self.portable.publish.assert_called_once()
        self.assertEqual(self.state.extension_protocols(self.portable.PORTABLE), {4})

    def test_desktop_rebuild_restores_missing_provenance(self):
        path = self.portable.PORTABLE / "share/release-sources.json"
        old = json.loads(path.read_text())
        del old["desktop_protocol"]
        del old["desktop_features"]
        path.write_text(json.dumps(old))
        self.run_main(["--components", "desktop"])
        self.portable.publish.assert_called_once()
        saved = self.state.read_record(self.portable.PORTABLE)
        self.assertEqual(saved["desktop_protocol"], 4)
        self.assertEqual(saved["desktop_features"], [])

    def test_unrepaired_missing_provenance_rejected(self):
        path = self.portable.PORTABLE / "share/release-sources.json"
        original = json.loads(path.read_text())
        for key, message in (("desktop_protocol", "protocol provenance"),
                             ("desktop_features", "supported-feature provenance")):
            for args in (["--components", "core"], ["--reuse-binaries"]):
                with self.subTest(key=key, args=args):
                    old = dict(original)
                    del old[key]
                    path.write_text(json.dumps(old))
                    self.assert_rejected_unchanged(args, message)

    def test_reuse_checks_shipped_metadata_not_checkout(self):
        self.protocol = 9
        self.features = [FEATURE]
        self.set_native()
        self.run_main(["--reuse-binaries"])
        self.portable.resolve.assert_not_called()
        self.portable.install_desktop.assert_not_called()
        self.portable.install_core.assert_not_called()
        self.portable.run.assert_not_called()
        self.portable.publish.assert_called_once()
        self.assertEqual(self.state.read_record(self.portable.PORTABLE)["desktop_protocol"], 4)

    def test_incompatible_reuse_is_rejected(self):
        extension_files(self.portable.PORTABLE / "share/extensions", 4, [FEATURE])
        self.assert_rejected_unchanged(["--reuse-binaries"], "unsupported.*" + FEATURE)
        self.portable.resolve.assert_not_called()
        self.portable.run.assert_not_called()

    def test_extensions_output_is_checked_after_successful_preflight(self):
        self.shipped_required = [FEATURE]
        self.assert_rejected_unchanged(["--components", "extensions"], "unsupported.*" + FEATURE)
        self.portable.run.assert_called_once()

    @contextlib.contextmanager
    def copied_from(self, origin):
        copier = self.portable.shutil.copytree
        copy2 = self.portable.shutil.copy2
        reads = {}

        def observed_file(source, target):
            path = Path(source)
            reads[path.relative_to(origin).as_posix()] = path.stat().st_size
            return copy2(source, target)

        def observed_tree(source, target, *args, **kwargs):
            if Path(source) == origin:
                kwargs["copy_function"] = observed_file
            return copier(source, target, *args, **kwargs)

        with patch.object(self.portable.shutil, "copytree", side_effect=observed_tree):
            yield reads

    def test_extensions_only_does_not_copy_payload_replaced_by_bundle(self):
        base = self.root / "ready-base"
        seed(base, self.state)
        old_payload = "share/extensions/sample/old.asset"
        dropped = base / old_payload
        dropped.write_bytes(b"old payload" * 65536)
        kept = "share/modules/sample/extensions/kept"
        put(base / kept, "retained module data")
        (base / kept).chmod(0o640)
        link = "share/module-alias"
        (base / link).symlink_to("modules/sample/extensions/kept")
        before = snapshot(base)

        with self.copied_from(base) as reads:
            self.run_main(["--components", "extensions", "--base", str(base)])

        stage = self.portable.PORTABLE
        self.assertNotIn(old_payload, reads)
        self.assertFalse(any(name.startswith("share/extensions/") for name in reads))
        self.assertEqual(reads[kept], len(b"retained module data"))
        self.assertFalse((stage / old_payload).exists())
        self.assertTrue((stage / link).is_symlink())
        self.assertEqual((stage / link).readlink(), (base / link).readlink())
        after = snapshot(stage)
        self.assertEqual(after[kept], before[kept])
        self.assertEqual(snapshot(base), before)
        self.assertEqual(self.state.extension_protocols(stage), {4})
        saved = self.state.read_record(stage)
        self.assertEqual(saved["build_components"], ["extensions"])
        self.assertIn("share/extensions/sample/bin/extension", saved["binary_sha256"])
        self.assertEqual(saved["sources"]["Proteus-desktop-extensions"]["commit"], "synthetic-new")
        self.portable.install_desktop.assert_not_called()
        self.portable.install_core.assert_not_called()
        self.portable.publish.assert_called_once()

    def test_old_subtree_is_copied_for_reuse_mixed_or_aliased_base(self):
        base = self.root / "ready-base"
        seed(base, self.state)
        alias = self.root / "alias"
        alias.symlink_to(base, target_is_directory=True)
        old = "share/extensions/sample/old.asset"
        put(base / old, "old bytes")
        for selection, requested_base in [
            (["--reuse-binaries"], base),
            (["--components", "desktop", "extensions"], base),
            (["--components", "extensions"], alias),
        ]:
            with self.subTest(selection=selection, base=requested_base.name):
                with self.copied_from(base) as reads:
                    self.run_main([*selection, "--base", str(requested_base)])
                self.assertEqual(reads[old], len(b"old bytes"))

    def test_linked_or_nonordinary_subtree_keeps_legacy_copy_without_bundling(self):
        ignore = self.portable._ignore_replaced_extensions
        base = self.root / "ready-base"
        seed(base, self.state)
        share = base / "share"
        ext = share / "extensions"
        renamed = self.root / "real-extensions"
        ext.rename(renamed)
        ext.symlink_to(renamed, target_is_directory=True)
        self.assertIsNone(ignore(base))

        def stop_before_bundle(stage, *_):
            staged = stage / "share/extensions"
            self.assertTrue(staged.is_symlink())
            self.assertEqual(staged.readlink(), renamed)
            raise SystemExit("inert stop before bundle")

        with patch.object(self.portable, "assemble", side_effect=stop_before_bundle):
            self.assert_rejected_unchanged(
                ["--components", "extensions", "--base", str(base)], "inert stop before bundle",
            )
        ext.unlink()
        renamed.rename(ext)
        ext.chmod(0o500)
        self.assertIsNone(ignore(base))
        ext.chmod(0o700)
        real_share = self.root / "real-share"
        share.rename(real_share)
        share.symlink_to(real_share, target_is_directory=True)
        self.assertIsNone(ignore(base))
        self.portable.run.assert_not_called()

    def test_extensions_bundle_failure_preserves_destination_and_cleans_stage(self):
        self.portable.run.side_effect = SystemExit("inert extension bundle failed")
        self.assert_rejected_unchanged(["--components", "extensions"], "inert extension bundle failed")
        self.portable.run.assert_called_once()

    def test_extensions_only_checks_old_ready_base_before_copy(self):
        old = self.portable.PORTABLE / "share/extensions/sample/bin/extension"
        old.unlink()
        self.assert_rejected_unchanged(["--components", "extensions"], "missing executable")
        self.portable.run.assert_not_called()

    def test_plan_never_validates_or_publishes(self):
        before = snapshot(self.root)
        self.portable.read_record = forbidden
        self.run_main(["--full", "--plan"])
        self.assertEqual(snapshot(self.root), before)
        self.portable.resolve.assert_not_called()
        self.portable.publish.assert_not_called()
        self.portable.run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
