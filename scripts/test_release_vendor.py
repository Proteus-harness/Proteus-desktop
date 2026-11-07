"""Vendor test harnesses compile the exact checkout without editing manifests."""
import tempfile
from pathlib import Path
import tomllib
import unittest

from test_vendor import manifest


class VendorHarnessTests(unittest.TestCase):
    def test_nonstandard_lib_path_and_private_lock_are_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
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
