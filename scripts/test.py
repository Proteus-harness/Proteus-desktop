#!/usr/bin/env python3
"""Prepare separate extension sources and run the client workspace gate."""
import os
import subprocess
import sys
from sources import ROOT, resolve
from limits import limit
from check_licenses import check

limit()

env = os.environ.copy()
extensions = resolve("extensions")
env["PROTEUS_TEST_EXTENSIONS_SOURCE"] = str(extensions)
env["PROTEUS_EXTENSIONS_DIR"] = str(extensions / "packages")
env.setdefault("RUST_TEST_THREADS", "4")
problems = check(ROOT) + check(extensions)
if problems:
    raise SystemExit("\n".join(problems))
# Packages are Rust binaries; `--dev` places them next to their manifests.
subprocess.run([sys.executable, str(extensions / "package.py"), "--dev"], check=True, env=env)
raise SystemExit(subprocess.call(["cargo", "test", "--locked", "--workspace", *sys.argv[1:]], cwd=ROOT, env=env))
