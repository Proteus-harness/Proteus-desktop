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


if __name__ == "__main__":
    unittest.main()
