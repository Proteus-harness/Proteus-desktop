#!/usr/bin/env python3
"""Builds the portable native client: build/Proteus.

The folder runs without the repository: the client, the agent server, the
reference module and the tracked profiles sit next to a launcher. A tar.gz
of the folder is written next to it.
"""

import argparse
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
from sources import resolve
from limits import limit
from portable_dcp import assemble as assemble_dcp
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


def assemble(agent_profile, client_profile):
    staging = BUILD / "Proteus.next"
    shutil.rmtree(staging, ignore_errors=True)
    agent = ROOT / "target" / agent_profile
    module = MODULES / "target" / agent_profile
    client = NATIVE / "target" / client_profile
    install_file(client / "proteus-native", staging / "bin/proteus-native", True)
    install_file(agent / "proteus", staging / "bin/proteus", True)
    install_file(
        module / "proteus-reference-module",
        staging / "bin/proteus-reference-module",
        True,
    )
    assemble_dcp(MODULES, staging)
    assemble_modules(ROOT, MODULES, staging)
    for path in tracked_configs():
        install_file(ROOT / path, staging / "share" / path)
    run([sys.executable, str(EXTENSIONS / "package.py"), "--bundle",
         str(staging / "share/extensions")], ROOT)
    install_notices(staging, NATIVE, ROOT, MODULES, EXTENSIONS)
    write_manifest(staging, NATIVE, ROOT, MODULES, EXTENSIONS)
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
    args = parser.parse_args()
    limit()
    global ROOT, MODULES, EXTENSIONS
    ROOT, MODULES, EXTENSIONS = resolve("harness"), resolve("modules"), resolve("extensions")
    profile = "debug" if args.debug else "release"
    flags = [] if args.debug else ["--release"]
    run(
        ["cargo", "build", "--locked", *flags, "-p", "proteus-core"],
        ROOT,
    )
    run(["cargo", "build", "--locked", *flags, "-p", "proteus-reference-module"], MODULES)
    client_profile = "fast" if args.fast else profile
    client_flags = ["--profile", "fast"] if args.fast else flags
    run(["cargo", "build", "--locked", *client_flags, "-p", "proteus-native"], NATIVE)
    assemble(profile, client_profile)
    print(f"Portable: {PORTABLE}")
    if not (args.no_archive or args.fast):
        print(f"Archive:  {archive()}")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
