"""Copy tracked standalone module assets and their relocatable launchers."""
import hashlib
import json
import shutil
import stat
import subprocess
from pathlib import Path


def require_node():
    version = subprocess.check_output(
        ["node", "-p", "process.versions.node"], text=True,
    ).strip()
    if tuple(map(int, version.split(".")[:2])) < (22, 18):
        raise SystemExit("Portable hook modules require Node.js 22.18+ on PATH")


def copy_tracked(repository, source, target):
    source = Path(source)
    tracked = subprocess.check_output(
        ["git", "ls-files", "-z", "--", str(source)], cwd=repository,
    )
    if not tracked:
        raise SystemExit(f"No tracked module sources found in {repository / source}")
    for name in tracked.split(b"\0"):
        if name:
            relative = Path(name.decode())
            destination = target / relative.relative_to(source)
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(repository / relative, destination)


def install_launcher(portable, name, entry):
    wrapper = portable / "bin" / name
    wrapper.parent.mkdir(parents=True, exist_ok=True)
    wrapper.write_text(
        '#!/bin/sh\nset -eu\n'
        'here=$(dirname "$(readlink -f "$0")")\n'
        f'exec node "${{here}}/../{entry}" "$@"\n'
    )
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def assemble(core, modules, portable):
    """Ship hook sources/SDK without installing or enabling any hook profile."""
    require_node()
    adapter = portable / "share/modules/claude-command-hooks"
    sdk = portable / "share/modules/hook-process"
    copy_tracked(modules, "hook/claude-command-hooks", adapter)
    copy_tracked(core, "examples/modules/hook-process", sdk)
    for repository, target in ((modules, adapter), (core, sdk)):
        shutil.copy2(repository / "LICENSE", target / "LICENSE")
    pin = json.loads((adapter / "sdk-pin.json").read_text())
    for name, expected in pin["sha256"].items():
        if hashlib.sha256((sdk / name).read_bytes()).hexdigest() != expected:
            raise SystemExit(f"Portable hook SDK pin mismatch: {name}")
    install_launcher(portable, "proteus-hook-worker", "share/modules/hook-process/worker.mjs")
    install_launcher(portable, "proteus-claude-hooks-inspect", "share/modules/claude-command-hooks/inspect.mjs")
