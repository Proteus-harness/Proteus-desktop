#!/usr/bin/env python3
"""Builds the portable native client: build/Proteus.

The folder runs without the repository: the client, the agent server, the
independent modules and the tracked profiles sit next to a launcher. A tar.gz
of the folder is written next to it.
"""

import argparse
import hashlib
import json
import os
import shutil
import stat
import subprocess
import sys
import tarfile
from pathlib import Path

NATIVE = Path(__file__).resolve().parent.parent
ROOT = None
MODULES = None
EXTENSIONS = None
REUSE_HOSTS = False
from sources import resolve
from limits import limit
from cargo_binary import build_binary, existing_binary
from portable_modules import assemble as assemble_modules
from release_metadata import install_notices, version, write_manifest
BUILD = NATIVE / "build"
PORTABLE = BUILD / "Proteus"
ASSETS = Path(__file__).resolve().parent / "portable"
ICON = NATIVE / "assets/icon.png"


def run(command, cwd):
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def tracked_configs():
    output = subprocess.run(
        ["git", "ls-files", "-z", "configs"],
        cwd=ROOT,
        check=True,
        capture_output=True,
    ).stdout
    return [Path(name.decode()) for name in output.split(b"\0") if name]


def install_file(source, target, executable=False):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)
    if executable:
        mode = target.stat().st_mode
        target.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def assemble(agent, client, module_profile="release", reuse_binaries=False):
    staging = BUILD / "Proteus.next"
    shutil.rmtree(staging, ignore_errors=True)
    install_file(client, staging / "bin/proteus-native", True)
    install_file(agent, staging / "bin/proteus", True)
    module_flags = ["--no-build"] if reuse_binaries else []
    run([sys.executable, str(MODULES / "package.py"), "--profile", module_profile,
         "--bundle", str(staging), *module_flags], MODULES)
    assemble_modules(ROOT, MODULES, staging)
    for path in tracked_configs():
        install_file(ROOT / path, staging / "share" / path)
    extension_flags = ["--no-build"] if reuse_binaries else []
    run([sys.executable, str(EXTENSIONS / "package.py"), "--bundle",
         str(staging / "share/extensions"), *extension_flags], EXTENSIONS)
    install_notices(staging, NATIVE, ROOT, MODULES, EXTENSIONS)
    record = write_manifest(staging, NATIVE, ROOT, MODULES, EXTENSIONS)
    if reuse_binaries or REUSE_HOSTS:
        record["build_mode"] = "reuse-binaries" if reuse_binaries else "reuse-hosts"
        record["binary_sha256"] = {}
        for binary in (staging / "bin").iterdir():
            if binary.is_file():
                with binary.open("rb") as stream:
                    record["binary_sha256"][binary.name] = hashlib.file_digest(stream, "sha256").hexdigest()
        (staging / "share/release-sources.json").write_text(json.dumps(record, indent=2) + "\n")
    install_file(ICON, staging / "share/icons/proteus-native.png")
    install_file(NATIVE / "assets/proteus-logo.svg", staging / "share/icons/proteus-native.svg")
    install_file(ASSETS / "proteus-native", staging / "proteus-native", True)
    install_file(ASSETS / "system_proxy.py", staging / "share/system_proxy.py")
    install_file(ASSETS / "install-shortcut", staging / "install-shortcut", True)
    shutil.rmtree(PORTABLE, ignore_errors=True)
    staging.rename(PORTABLE)


def archive():
    machine = os.uname().machine
    path = BUILD / f"Proteus-{version(NATIVE)}-linux-{machine}.tar.gz"
    with tarfile.open(path, "w:gz") as tar:
        tar.add(PORTABLE, arcname="Proteus")
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--debug",
        action="store_true",
        help="use debug builds (faster to build, slower to run)",
    )
    mode.add_argument(
        "--fast",
        action="store_true",
        help="build the client without LTO, incrementally, and skip the archive "
        "(for local rebuilds; runs nearly as fast as release)",
    )
    parser.add_argument(
        "--no-archive", action="store_true", help="skip the tar.gz archive"
    )
    parser.add_argument(
        "--reuse-binaries", action="store_true",
        help="assemble profiles, modules and extensions using existing binaries; skip Cargo builds",
    )
    parser.add_argument(
        "--reuse-hosts", action="store_true",
        help="build module/extension packages while reusing existing Core and Desktop binaries",
    )
    args = parser.parse_args()
    if not args.reuse_binaries:
        limit()
    global ROOT, MODULES, EXTENSIONS, REUSE_HOSTS
    REUSE_HOSTS = args.reuse_hosts
    ROOT, MODULES, EXTENSIONS = resolve("harness"), resolve("modules"), resolve("extensions")
    profile = "debug" if args.debug else "release"
    flags = [] if args.debug else ["--release"]
    client_profile = "fast" if args.fast else profile
    client_flags = ["--profile", "fast"] if args.fast else flags
    if args.reuse_binaries or args.reuse_hosts:
        agent = existing_binary(ROOT, profile, "proteus")
        client = existing_binary(NATIVE, client_profile, "proteus-native")
    else:
        agent = build_binary(ROOT, "proteus-core", "proteus", flags,
                             ROOT / "crates/proteus-core/Cargo.toml")
        client = build_binary(NATIVE, "proteus-native", "proteus-native", client_flags,
                              NATIVE / "Cargo.toml")
    assemble(agent, client, "dev" if args.debug else "release", reuse_binaries=args.reuse_binaries)
    print(f"Portable: {PORTABLE}")
    if not (args.no_archive or args.fast):
        print(f"Archive:  {archive()}")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
