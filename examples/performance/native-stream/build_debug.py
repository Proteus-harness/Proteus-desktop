#!/usr/bin/env python3
"""Build the diagnostic Desktop with the same GPUI feature set as its tests."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "scripts"))
from limits import limit
from desktop_toolchain import prepare_desktop

if __name__ == "__main__":
    limit()
    raise SystemExit(subprocess.call([
        "cargo", "build", "--locked", "-p", "proteus-native", "--bin", "proteus-native",
        "--all-targets", "--features", "gpui/test-support",
    ], cwd=ROOT, env=prepare_desktop(ROOT)))
