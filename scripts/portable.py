#!/usr/bin/env python3
"""Update selected portable components; default builds Desktop only.

Unselected components are copied intact from --base (default build/Proteus).
An explicit --full bootstraps all components. --plan never builds or clones.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile

from sources import resolve
from limits import limit
from cargo_binary import build_binary
from portable_modules import assemble as assemble_modules
from portable_state import (COMPONENTS, SOURCE_NAMES, binary_hashes, clear_modules,
                            desktop_protocol, publish, read_record, verify_extension_source, verify_protocol)
from release_metadata import install_component_notices, source_record, version, write_manifest

NATIVE = Path(__file__).resolve().parent.parent
BUILD = NATIVE / "build"
PORTABLE = BUILD / "Proteus"
ASSETS = Path(__file__).resolve().parent / "portable"


def run(command, cwd):
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, check=True)


def install_file(source, target, executable=False):
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)
    if executable:
        target.chmod(target.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def install_desktop(stage, flags):
    client = build_binary(NATIVE, "proteus-native", "proteus-native", flags, NATIVE / "Cargo.toml")
    install_file(client, stage / "bin/proteus-native", True)
    shutil.rmtree(stage / "share/icons", ignore_errors=True)
    install_file(NATIVE / "assets/icon.png", stage / "share/icons/proteus-native.png")
    install_file(NATIVE / "assets/proteus-logo.svg", stage / "share/icons/proteus-native.svg")
    for name in ("proteus-native", "install-shortcut"):
        install_file(ASSETS / name, stage / name, True)
    install_file(ASSETS / "system_proxy.py", stage / "share/system_proxy.py")


def install_core(stage, core, flags):
    agent = build_binary(core, "proteus-core", "proteus", flags,
                         core / "crates/proteus-core/Cargo.toml")
    install_file(agent, stage / "bin/proteus", True)
    shutil.rmtree(stage / "share/configs", ignore_errors=True)
    output = subprocess.run(["git", "ls-files", "-z", "configs"], cwd=core,
                            check=True, capture_output=True).stdout
    for name in output.split(b"\0"):
        if name:
            path = Path(name.decode())
            install_file(core / path, stage / "share" / path)


def assemble(stage, selected, roots, profile, base_record=None):
    flags = [] if profile == "debug" else ["--release"]
    for component in selected:
        if component == "desktop":
            install_desktop(stage, flags)
        elif component == "core":
            install_core(stage, roots["core"], flags)
        elif component == "modules":
            clear_modules(stage)
            modules = roots["modules"]
            run([sys.executable, str(modules / "package.py"), "--profile",
                 "dev" if profile == "debug" else "release", "--bundle", str(stage)], modules)
            # Hook SDK is copied as source; selecting modules never compiles Core.
            assemble_modules(roots["core"], modules, stage)
        elif component == "extensions":
            shutil.rmtree(stage / "share/extensions", ignore_errors=True)
            extensions = roots["extensions"]
            run([sys.executable, str(extensions / "package.py"), "--bundle",
                 str(stage / "share/extensions")], extensions)
        install_component_notices(stage, SOURCE_NAMES[component], roots[component])
    if base_record is None:
        # Official full release metadata keeps exactly the release.py manifest schema.
        write_manifest(stage, NATIVE, roots["core"], roots["modules"], roots["extensions"])
        from release_metadata import NOTICE_README
        (stage / "share/licenses/README.txt").write_text(NOTICE_README)
    else:
        record = copy.deepcopy(base_record)
        record["build_mode"] = "partial"
        record["build_components"] = list(selected)
        record["binary_sha256"] = binary_hashes(stage)
        for component in selected:
            record["sources"][SOURCE_NAMES[component]] = source_record(
                SOURCE_NAMES[component], roots[component])
        if "modules" in selected:
            record["module_sdk_source"] = source_record("Proteus-core", roots["core"])
        if "desktop" in selected:
            record["version"] = version(NATIVE)
            record["desktop_protocol"] = desktop_protocol(NATIVE)
        (stage / "share/release-sources.json").write_text(json.dumps(record, indent=2) + "\n")


def archive():
    packaged = json.loads((PORTABLE / "share/release-sources.json").read_text())["version"]
    path = BUILD / f"Proteus-{packaged}-linux-{os.uname().machine}.tar.gz"
    with tarfile.open(path, "w:gz") as tar:
        tar.add(PORTABLE, arcname="Proteus")
    return path


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    scope = result.add_mutually_exclusive_group()
    scope.add_argument("--components", nargs="+", choices=COMPONENTS,
                       help="components to build (default: desktop)")
    scope.add_argument("--full", action="store_true", help="explicitly build all four components")
    scope.add_argument("--reuse-binaries", action="store_true",
                       help="copy a complete ready --base intact; no builds or source resolution")
    result.add_argument("--base", type=Path, default=None, help="ready portable (default: build/Proteus)")
    result.add_argument("--debug", action="store_true", help="use debug instead of ordinary release builds")
    result.add_argument("--no-archive", action="store_true", help="skip tar.gz")
    result.add_argument("--plan", action="store_true", help="print selected build/reuse scope only")
    return result


def main(argv=None):
    arguments = parser()
    args = arguments.parse_args(argv)
    if args.reuse_binaries and args.debug:
        arguments.error("--debug cannot be combined with --reuse-binaries (ready binaries stay unchanged)")
    if args.components and len(set(args.components)) != len(args.components):
        arguments.error("--components must be distinct")
    selected = tuple(COMPONENTS if args.full else args.components or ("desktop",))
    if args.reuse_binaries:
        selected = ()
    base = (args.base or PORTABLE).resolve()
    print(f"Base: {base}")
    for component in COMPONENTS:
        print(f"{component}: {'build' if component in selected else 'reuse ready base'}")
    if args.plan:
        return
    complete = set(selected) == set(COMPONENTS)
    record = None if complete else read_record(base)
    if selected and not complete:
        verify_protocol(base, NATIVE, selected, record)
    roots = {"desktop": NATIVE}
    # Resolve only chosen sources; modules also need read-only hook SDK sources.
    if "core" in selected or "modules" in selected:
        roots["core"] = resolve("harness")
    if "modules" in selected:
        roots["modules"] = resolve("modules")
    if "extensions" in selected:
        roots["extensions"] = resolve("extensions")
    if "extensions" in selected and "desktop" not in selected:
        verify_extension_source(base, roots["extensions"], record)
    if selected:
        limit()
    BUILD.mkdir(parents=True, exist_ok=True)
    stage = Path(tempfile.mkdtemp(prefix="Proteus.next-", dir=BUILD))
    try:
        if not complete:
            shutil.copytree(base, stage, dirs_exist_ok=True, symlinks=True)
        if selected:
            assemble(stage, selected, roots, "debug" if args.debug else "release", record)
        # Reject incomplete selected packaging before replacing a working installation.
        read_record(stage)
        publish(stage, PORTABLE)
    finally:
        if stage.exists():
            shutil.rmtree(stage)
    print(f"Portable: {PORTABLE}")
    if not args.no_archive:
        print(f"Archive:  {archive()}")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
