"""The installed application shortcut follows ordinary last-workspace startup."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class ShortcutTests(unittest.TestCase):
    def test_install_does_not_force_folder_picker(self):
        source = Path(__file__).parent / "portable" / "install-shortcut"
        with tempfile.TemporaryDirectory(prefix="proteus-shortcut-test-") as temporary:
            base = Path(temporary)
            portable = base / "portable folder"
            portable.mkdir()
            installer = portable / "install-shortcut"
            installer.write_bytes(source.read_bytes())
            icon = portable / "share/icons/proteus-native.svg"
            icon.parent.mkdir(parents=True)
            icon.write_text("<svg/>")
            env = {**os.environ, "XDG_DATA_HOME": str(base / "data")}
            subprocess.run(["sh", str(installer)], check=True, env=env, capture_output=True)
            entry = base / "data/applications/dev.proteus.native.desktop"
            text = entry.read_text()
            self.assertIn(f'Exec="{portable}/proteus-native"\n', text)
            self.assertNotIn("--choose-workspace", text)
            subprocess.run(["sh", str(installer), "--remove"], check=True, env=env, capture_output=True)
            self.assertFalse(entry.exists())

    def test_launcher_adds_missing_assets_and_preserves_restored_fragments(self):
        source = Path(__file__).parent / "portable" / "proteus-native"
        with tempfile.TemporaryDirectory(prefix="proteus restore test ") as temporary:
            base = Path(temporary)
            launcher = base / "proteus-native"
            launcher.write_bytes(source.read_bytes())
            bundled = base / "share/configs/fragments"
            bundled.mkdir(parents=True)
            (bundled / "codex.toml").write_text('mode = "bundled"\n')
            (bundled / "new.toml").write_text('mode = "new"\n')
            config = base / "custom XDG/Proteus-harness"
            existing = config / "configs/fragments"
            existing.mkdir(parents=True)
            (existing / "codex.toml").write_text('mode = "restored"\n')
            (base / "share/system_proxy.py").write_text("import os\nprint(os.environ['PROTEUS_CONFIG_HOME'])\n")

            env = {**os.environ, "XDG_CONFIG_HOME": str(config.parent)}
            env.pop("PROTEUS_CONFIG_HOME", None)
            result = subprocess.run(["sh", str(launcher)], env=env, check=True, capture_output=True, text=True)
            self.assertEqual(result.stdout.strip(), str(config))
            self.assertEqual((existing / "codex.toml").read_text(), 'mode = "restored"\n')
            self.assertEqual((existing / "new.toml").read_text(), 'mode = "new"\n')


if __name__ == "__main__":
    unittest.main()
