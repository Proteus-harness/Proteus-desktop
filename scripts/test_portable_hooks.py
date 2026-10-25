#!/usr/bin/env python3
"""Probe packaged hook assets after relocation, without Cargo or source builds."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile


def probe(portable):
    with tempfile.TemporaryDirectory(prefix="proteus-portable-hooks-") as temporary:
        work = Path(temporary)
        staging = work / "original"
        # Copy only the delivered hook assets; no repository or build dependency.
        for name in ("claude-command-hooks", "hook-process"):
            shutil.copytree(portable / "share/modules" / name, staging / "share/modules" / name)
        (staging / "bin").mkdir()
        for name in ("proteus-hook-worker", "proteus-claude-hooks-inspect"):
            shutil.copy2(portable / "bin" / name, staging / "bin" / name)
        relocated = work / "relocated Proteus"
        staging.rename(relocated)
        adapter = relocated / "share/modules/claude-command-hooks"
        sdk = relocated / "share/modules/hook-process"
        for path in (
            adapter / "LICENSE", adapter / "PROVENANCE.md", adapter / "sdk-pin.json",
            adapter / "README.md", sdk / "LICENSE", sdk / "README.md", sdk / "hooks.d.mts",
        ):
            assert path.is_file(), f"Missing hook source/license: {path}"
        cwd = work / "unrelated workspace"
        cwd.mkdir()
        marker = work / "must-not-exist"
        config = work / "foreign-hooks.json"
        config.write_text(json.dumps({"hooks": {"PreToolUse": [{
            "matcher": "Write", "hooks": [{
                "type": "command", "command": "node", "args": ["-e",
                    "require('node:fs').writeFileSync(" + json.dumps(str(marker)) + ", 'executed')"],
            }],
        }]}}))
        settings = work / "settings.json"
        settings.write_text(json.dumps({
            "mode": "strict_subset", "allow_execution": False,
            "config_path": str(config), "sdk_root": str(sdk), "events": ["PreToolUse"],
        }))
        inspected = subprocess.run(
            [str(relocated / "bin/proteus-claude-hooks-inspect"), str(settings)],
            cwd=cwd, check=True, capture_output=True, text=True, timeout=10,
        )
        assert json.loads(inspected.stdout)["compatible"] is True
        assert not marker.exists(), "Inspector executed a command hook"

        # Check the generic launcher itself; bundled Node tests cover invocation,
        # command execution, cancellation and the adapter's worker protocol.
        entry = work / "generic-entry.mjs"
        entry.write_text("export default function setup(hooks) {}\n")
        frame = {
            "jsonrpc": "2.0", "id": "h:1:0", "method": "initialize",
            "params": {"protocol_version": "v3", "component_id": "generic-hooks", "exports": [{
                "slot": "hook", "module_id": "generic", "contract_version": "v6",
                "composition": "ordered_many", "host_features": [],
                "module_config": {"entry": str(entry)},
            }]},
        }
        initialized = subprocess.run(
            [str(relocated / "bin/proteus-hook-worker")], input=json.dumps(frame) + "\n",
            cwd=cwd, check=True, capture_output=True, text=True, timeout=10,
        )
        response = json.loads(initialized.stdout)
        assert response["id"] == frame["id"], response
        assert response["result"]["component_id"] == "generic-hooks", response
        assert response["result"]["exports"][0]["module_id"] == "generic", response

        env = os.environ.copy()
        env["PROTEUS_HOOK_SDK_ROOT"] = str(sdk)
        tests = sorted((adapter / "tests").glob("*.test.mjs"))
        assert tests, "Missing bundled adapter tests"
        subprocess.run(["node", "--test", *map(str, tests)], cwd=cwd, env=env, check=True, timeout=60)
    print("PASS: relocated hook launchers, sources/licenses, inspection without execution and adapter Node gate")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: python3 scripts/test_portable_hooks.py /path/to/Proteus")
    probe(Path(sys.argv[1]).resolve())
