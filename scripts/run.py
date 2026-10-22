#!/usr/bin/env python3
"""Run the development client with its separately maintained default packages."""
import os
import subprocess
import sys
from sources import ROOT, resolve

env = os.environ.copy()
extensions = resolve("extensions")
# Packages are Rust binaries; `--dev` places them next to their manifests.
subprocess.run([sys.executable, str(extensions / "package.py"), "--dev"], check=True)
env["PROTEUS_EXTENSIONS_DIR"] = str(extensions / "packages")
os.chdir(ROOT)
os.execvpe("cargo", ["cargo", "run", "--locked", "-p", "proteus-native", "--", *sys.argv[1:]], env)
