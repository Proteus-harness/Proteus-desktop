#!/usr/bin/env python3
"""Prepare separate extension sources and run the client workspace gate."""
import os
import subprocess
import sys
from sources import ROOT, resolve

env = os.environ.copy()
extensions = resolve("extensions")
env["PROTEUS_TEST_EXTENSIONS_SOURCE"] = str(extensions)
env["PROTEUS_EXTENSIONS_DIR"] = str(extensions / "packages")
env.setdefault("CARGO_BUILD_JOBS", "2")
env.setdefault("RUST_TEST_THREADS", "4")
raise SystemExit(subprocess.call(["cargo", "test", "--locked", "--workspace", *sys.argv[1:]], cwd=ROOT, env=env))
