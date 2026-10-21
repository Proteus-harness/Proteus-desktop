"""Bundle the standalone DCP process without modifying its source checkout."""
import shutil
import stat
import subprocess
from pathlib import Path


def assemble(modules, portable):
    """Build DCP and its locked dependencies inside the new portable folder."""
    version = subprocess.run(
        ["node", "-p", "process.versions.node.split('.')[0]"],
        check=True, capture_output=True, text=True,
    ).stdout.strip()
    if int(version) < 22:
        raise SystemExit("Portable DCP requires Node.js 22+ (and npm) on PATH")
    source = Path("hook/dcp")
    target = portable / "share/modules/dcp"
    tracked = subprocess.run(
        ["git", "ls-files", "-z", "--", str(source)],
        cwd=modules, check=True, capture_output=True,
    ).stdout
    if not tracked:
        raise SystemExit(f"No tracked DCP sources found in {modules / source}")
    # Keep the corresponding sources, package lock, tests and all module notices.
    # Production node_modules retain upstream sources and dependency licenses.
    for name in tracked.split(b"\0"):
        if not name:
            continue
        relative = Path(name.decode())
        destination = target / relative.relative_to(source)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(modules / relative, destination)
    for command in (
        ["npm", "ci", "--ignore-scripts"],
        ["npm", "run", "build"],
        ["npm", "prune", "--omit=dev", "--ignore-scripts"],
    ):
        print("+", " ".join(command), flush=True)
        subprocess.run(command, cwd=target, check=True)
    wrapper = portable / "bin/proteus-dcp"
    wrapper.parent.mkdir(parents=True, exist_ok=True)
    wrapper.write_text(
        '#!/bin/sh\nset -eu\n'
        'here=$(dirname "$(readlink -f "$0")")\n'
        'exec node "${here}/../share/modules/dcp/dist/worker.js" "$@"\n'
    )
    wrapper.chmod(wrapper.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
