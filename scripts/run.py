#!/usr/bin/env python3
"""Run the development client with its separately maintained default packages."""
import os
import sys
from sources import ROOT, resolve

env = os.environ.copy()
env["PROTEUS_EXTENSIONS_DIR"] = str(resolve("extensions") / "packages")
os.chdir(ROOT)
os.execvpe("cargo", ["cargo", "run", "--locked", "-p", "proteus-native", "--", *sys.argv[1:]], env)
