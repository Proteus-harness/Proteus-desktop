"""Real Cargo artifact selection using a tiny offline crate, without building GPUI."""
import contextlib
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from cargo_binary import build_binary


class CargoArtifactIntegrationTests(unittest.TestCase):
    def test_custom_profile_and_relative_external_cache_ignore_stale_output(self):
        with tempfile.TemporaryDirectory(prefix="proteus cargo integration ") as directory:
            checkout = Path(directory) / "checkout"
            (checkout / "src").mkdir(parents=True)
            manifest = checkout / "Cargo.toml"
            manifest.write_text(
                '[package]\nname = "proteus-artifact-fixture"\nversion = "0.1.0"\n'
                'edition = "2024"\n[profile.fast]\ninherits = "dev"\n'
            )
            (checkout / "Cargo.lock").write_text(
                'version = 4\n[[package]]\nname = "proteus-artifact-fixture"\nversion = "0.1.0"\n'
            )
            (checkout / "src/main.rs").write_text('fn main() { println!("fresh artifact"); }\n')
            stale = checkout / "target/fast/proteus-artifact-fixture"
            stale.parent.mkdir(parents=True)
            stale.write_bytes(b"stale output, not executable")
            with patch.dict(os.environ, {"CARGO_TARGET_DIR": "../cache with spaces"}), \
                 contextlib.redirect_stdout(io.StringIO()):
                binary = build_binary(
                    checkout, "proteus-artifact-fixture", "proteus-artifact-fixture",
                    ["--offline", "--profile", "fast"], manifest,
                )
            self.assertTrue(binary.resolve().is_relative_to(Path(directory) / "cache with spaces"))
            self.assertEqual(binary.parent.name, "fast")
            self.assertEqual(binary.name, "proteus-artifact-fixture")
            result = subprocess.run([str(binary)], check=True, capture_output=True, text=True)
            self.assertEqual(result.stdout, "fresh artifact\n")
            self.assertEqual(stale.read_bytes(), b"stale output, not executable")


if __name__ == "__main__":
    unittest.main()
