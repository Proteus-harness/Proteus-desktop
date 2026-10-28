#!/usr/bin/env python3
"""Probe DCP in an assembled portable folder, without Cargo or source builds."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile


def probe(portable):
    module = portable / "share/modules/dcp"
    for path in (
        "src/worker.mjs", "build.mjs", "package.json", "package-lock.json", "README.md",
        "dist/worker.js", "dist/module.js", "dist/LICENSE.upstream",
        "node_modules/@tarquinen/opencode-dcp/LICENSE",
        "node_modules/@tarquinen/opencode-dcp/lib/hooks.ts",
    ):
        assert (module / path).is_file(), f"Missing DCP runtime/source/license: {path}"
    assert not (module / "node_modules/esbuild").exists(), "Build-only esbuild was not pruned"
    def binding(slot, name):
        return {
            "slot": slot, "module_id": name, "contract_version": "v6",
            "composition": "ordered_many", "module_config": {}, "host_features": [],
        }
    frame = {
        "jsonrpc": "2.0", "id": "h:1:0", "method": "initialize",
        "params": {
            "protocol_version": "v3", "component_id": "dcp",
            "exports": [binding("hook", "hook.dcp"), binding("tool", "dcp.tools")],
        },
    }
    # Resolve the wrapper from an unrelated cwd, as Core does for workspace processes.
    with tempfile.TemporaryDirectory(prefix="proteus-portable-dcp-") as cwd:
        initialized = subprocess.run(
            [str(portable / "bin/proteus-dcp")], input=json.dumps(frame) + "\n",
            cwd=cwd, capture_output=True, text=True, check=True, timeout=20,
        )
    response = json.loads(initialized.stdout)
    assert response["id"] == frame["id"], response
    assert response["result"]["component_id"] == "dcp", response
    assert len(response["result"]["exports"]) == 2, response
    # These standalone tests execute real token counting/WASM, hook transforms,
    # and persistence after pruning development dependencies.
    subprocess.run(
        ["node", "--test", "tests/module.test.mjs"],
        cwd=module, check=True, timeout=60,
    )
    print("PASS: portable DCP wrapper initialization, sources/licenses, production runtime and WASM hooks")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: python3 scripts/test_portable_dcp.py /path/to/Proteus")
    probe(Path(sys.argv[1]).resolve())
