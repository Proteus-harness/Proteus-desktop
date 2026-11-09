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
import re
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


def source_links(source, destination):
    """Keep manifest-relative inputs attached to their original source files.

    Build scripts resolve relative watches against CARGO_MANIFEST_DIR, even
    when their own path is absolute. Stable links keep those watches valid and
    also expose package data such as locales, fixtures and GHOSTTY.lock.
    """
    entries = {entry.name: entry for entry in source.iterdir()
               if entry.name not in {"Cargo.toml", "Cargo.lock", "target", ".git"}}
    for entry in destination.iterdir():
        if entry.is_symlink() and entry.name not in entries:
            entry.unlink()
    for name, entry in entries.items():
        link = destination / name
        target = entry.absolute()
        if link.is_symlink():
            if link.readlink() == target:
                continue
            link.unlink()
        elif link.exists():
            raise ValueError(f"Refusing to replace a non-link in vendor projection: {link}")
        link.symlink_to(target, target_is_directory=entry.is_dir())


def development_profiles(original, profiles):
    """Share the host's dev/test settings without importing release overrides."""
    # Published manifests use ordinary table headings. Replace only the
    # projected dev/test tables; upstream source and release tables stay intact.
    lines, skip = [], False
    for line in original.splitlines(keepends=True):
        if re.match(r"\s*\[", line):
            skip = bool(re.match(r"\s*\[profile\.(?:dev|test)(?:\.|\])", line))
        if not skip:
            lines.append(line)

    def table(path, values):
        heading = ".".join(json.dumps(part) for part in path)
        result = [f"\n[{heading}]\n"]
        for key, value in values.items():
            if not isinstance(value, dict):
                result.append(f"{json.dumps(key)} = {json.dumps(value)}\n")
        for key, value in values.items():
            if isinstance(value, dict):
                result.extend(table((*path, key), value))
        return result

    for name in ("dev", "test"):
        if name in profiles:
            lines.extend(table(("profile", name), profiles[name]))
    return "".join(lines)


def manifest(package, root=ROOT):
    """Project the manifest and source links; compile the unmodified checkout."""
    source = root / "vendor" / package
    destination = root / "build/verification/vendor" / package
    destination.mkdir(parents=True, exist_ok=True)
    source_links(source, destination)
    original = (source / "Cargo.toml").read_text()
    metadata = tomllib.loads(original)
    lib = metadata["lib"].get("path", "src/lib.rs")
    original = original.replace('path = ' + json.dumps(lib),
                                'path = ' + json.dumps(str(source / lib)), 1)
    if isinstance(metadata["package"].get("build"), str):
        build = metadata["package"]["build"]
        original = original.replace('build = ' + json.dumps(build),
                                    'build = ' + json.dumps(str(source / build)), 1)
    host = tomllib.loads((root / "Cargo.toml").read_text())
    original = development_profiles(original, host.get("profile", {}))
    patches = host["patch"]["crates-io"]
    original += "\n[workspace]\n\n[patch.crates-io]\n"
    for name, value in patches.items():
        if name != package:
            original += f'{name} = {{ path = {json.dumps(str(root / value["path"]))} }}\n'
    path = destination / "Cargo.toml"
    if not path.exists() or path.read_text() != original:
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
