#!/usr/bin/env python3
"""Test excluded vendor crates against their actual sources and local patches.

Examples:
  python3 scripts/test_vendor.py ghostty-vt --full
  python3 scripts/test_vendor.py gpui-base --filter audit
  python3 scripts/test_vendor.py gpui-fast-wgpu --filter translucent_underline

A full run returns the real test status, including known baseline failures;
this runner never converts a failing vendor suite into a successful gate.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

from desktop_toolchain import prepare_desktop

ROOT = Path(__file__).resolve().parent.parent
FEATURES = {
    "ghostty-vt": [],
    "gpui-base": ["gpui-fast", "test-support"],
    "gpui-component": ["gpui-fast", "test-support", "tree-sitter"],
    "gpui-fast-wgpu": ["test-support"],
}


def manifest(package, root=ROOT):
    """Project just the manifest; compile the original, unmodified source tree."""
    source = root / "vendor" / package
    destination = root / "build/verification/vendor" / package
    destination.mkdir(parents=True, exist_ok=True)
    original = (source / "Cargo.toml").read_text()
    metadata = tomllib.loads(original)
    lib = metadata["lib"].get("path", "src/lib.rs")
    original = original.replace('path = ' + json.dumps(lib),
                                'path = ' + json.dumps(str(source / lib)), 1)
    if isinstance(metadata["package"].get("build"), str):
        build = metadata["package"]["build"]
        original = original.replace('build = ' + json.dumps(build),
                                    'build = ' + json.dumps(str(source / build)), 1)
    patches = tomllib.loads((root / "Cargo.toml").read_text())["patch"]["crates-io"]
    original += "\n[workspace]\n\n[patch.crates-io]\n"
    for name, value in patches.items():
        if name != package:
            original += f'{name} = {{ path = {json.dumps(str(root / value["path"]))} }}\n'
    path = destination / "Cargo.toml"
    path.write_text(original)
    lock = destination / "Cargo.lock"
    # Seed production dependency versions; Cargo adds upstream dev dependencies
    # to this private lock. Repeated runs retain the resolved test graph.
    if not lock.exists():
        shutil.copy2(root / "Cargo.lock", lock)
    return path


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("packages", nargs="+", choices=FEATURES)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--full", action="store_true", help="run all tests; baseline failures remain failures")
    mode.add_argument("--filter", help="run libtest cases matching this substring")
    parser.add_argument("--no-run", action="store_true", help="compile test targets without executing")
    parser.add_argument("--test-threads", type=int, help="libtest threads (default 1 for GPU, 4 otherwise)")
    parser.add_argument("--prepare-only", action="store_true", help="write manifests without invoking Cargo")
    args = parser.parse_args(argv)
    if args.test_threads is not None and args.test_threads < 1:
        parser.error("--test-threads must be positive")
    status = 0
    for package in dict.fromkeys(args.packages):
        path = manifest(package)
        command = ["cargo", "test", "--manifest-path", str(path),
                   "--target-dir", str(ROOT / "target"), "--lib"]
        if FEATURES[package]:
            command += ["--features", ",".join(FEATURES[package])]
        if args.no_run:
            command.append("--no-run")
        if args.filter:
            command.append(args.filter)
        if not args.no_run:
            command += ["--", f"--test-threads={args.test_threads or (1 if package == 'gpui-fast-wgpu' else 4)}"]
        print(json.dumps({"package": package, "manifest": str(path), "command": command}), flush=True)
        if args.prepare_only:
            continue
        env = dict(os.environ)
        env.setdefault("CARGO_BUILD_JOBS", "2")
        if package == "ghostty-vt":
            env = prepare_desktop(ROOT, environ=env)
        if package == "gpui-base" and args.full:
            print("Base audit baseline had 26 existing shared failures; a full run is not a green release gate.", file=sys.stderr)
        result = subprocess.run(command, cwd=ROOT, env=env)
        if result.returncode:
            status = result.returncode
    return status


if __name__ == "__main__":
    raise SystemExit(main())
