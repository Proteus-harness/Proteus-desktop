"""Source identity and notices shared by local portable and release packaging."""
import json
import platform
import re
import shutil
import subprocess
import tomllib
from pathlib import Path


_NUMBER = r"(?:0|[1-9][0-9]*)"
_IDENTIFIER = r"(?:0|[1-9][0-9]*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)"
VERSION_PATTERN = rf"{_NUMBER}\.{_NUMBER}\.{_NUMBER}(?:-{_IDENTIFIER}(?:\.{_IDENTIFIER})*)?"


def command(args, cwd):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def version(desktop):
    value = tomllib.loads((desktop / "Cargo.toml").read_text())["package"]["version"]
    if not re.fullmatch(VERSION_PATTERN, value):
        raise SystemExit(f"Invalid portable version: {value}")
    return value


def source_record(name, root):
    dirty = bool(command(["git", "status", "--porcelain", "--untracked-files=normal"], root))
    return {
        "repository": f"https://github.com/Proteus-harness/{name}.git",
        "commit": command(["git", "rev-parse", "HEAD"], root),
        "dirty": dirty,
    }


def manifest(desktop, core, modules, extensions):
    sources = {name: source_record(name, root) for name, root in (
        ("Proteus-desktop", desktop), ("Proteus-core", core),
        ("Proteus-modules", modules), ("Proteus-desktop-extensions", extensions),
    )}
    return {
        "schema_version": 1,
        "version": version(desktop),
        "platform": {"os": platform.system(), "architecture": platform.machine(),
                     "libc": list(platform.libc_ver())},
        "build_environment": {
            "rustc": command(["rustc", "--version"], desktop),
            "cargo": command(["cargo", "--version"], desktop),
            "node": command(["node", "--version"], desktop),
            "python": platform.python_version(),
        },
        "sources": sources,
        "runtime_requirements": {
            "node": ">=22", "python": ">=3", "git": "system",
            "ripgrep": "system", "bubblewrap": "Linux restricted profiles only",
            "desktop": "Linux graphical session and GPUI shared libraries",
        },
    }


def install_notices(portable, desktop, core, modules, extensions):
    target = portable / "share/licenses"
    for name, root in (("Proteus-desktop", desktop), ("Proteus-core", core),
                       ("Proteus-modules", modules), ("Proteus-desktop-extensions", extensions)):
        (target / name).mkdir(parents=True, exist_ok=True)
        shutil.copy2(root / "LICENSE", target / name / "LICENSE")
    for relative in ("tool/codex-patch", "hook/codex-context-preparation/src/upstream"):
        for name in ("LICENSE", "NOTICE"):
            source = modules / relative / name
            destination = target / "Proteus-modules" / relative / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
    (target / "README.txt").write_text(
        "Proteus source repositories are MIT licensed except where their local notices say otherwise.\n"
        "The Codex-derived code includes Apache-2.0 LICENSE and NOTICE files below.\n"
        "The separate DCP worker is AGPL-3.0-or-later. Its corresponding tracked sources,\n"
        "build instructions, package lock, upstream sources and dependency licenses are\n"
        "included in ../modules/dcp (including dist/LICENSE.upstream).\n"
        "The release sources archive also contains the four exact Proteus source snapshots.\n",
    )


def write_manifest(portable, desktop, core, modules, extensions):
    record = manifest(desktop, core, modules, extensions)
    destination = portable / "share/release-sources.json"
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text(json.dumps(record, indent=2) + "\n")
    return record
