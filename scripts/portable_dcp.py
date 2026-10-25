"""Bundle the standalone DCP process without modifying its source checkout."""
import subprocess
from pathlib import Path

from portable_modules import copy_tracked, install_launcher, require_node


def assemble(modules, portable):
    """Build DCP and its locked dependencies inside the new portable folder."""
    require_node()
    source = Path("hook/dcp")
    target = portable / "share/modules/dcp"
    # Keep the corresponding sources, package lock, tests and all module notices.
    # Production node_modules retain upstream sources and dependency licenses.
    copy_tracked(modules, source, target)
    for command in (
        ["npm", "ci", "--ignore-scripts"],
        ["npm", "run", "build"],
        ["npm", "prune", "--omit=dev", "--ignore-scripts"],
    ):
        print("+", " ".join(command), flush=True)
        subprocess.run(command, cwd=target, check=True)
    install_launcher(portable, "proteus-dcp", "share/modules/dcp/dist/worker.js")
