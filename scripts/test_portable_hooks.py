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
        name = "proteus-claude-hooks-inspect"
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

        env = os.environ.copy()
        env["PROTEUS_HOOK_SDK_ROOT"] = str(sdk)
        tests = sorted((adapter / "tests").glob("*.test.mjs"))
        assert tests, "Missing bundled adapter tests"
        subprocess.run(["node", "--test", *map(str, tests)], cwd=cwd, env=env, check=True, timeout=60)
    print("PASS: relocated hook assets, sources/licenses, inspection without execution and adapter Node gate")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: python3 scripts/test_portable_hooks.py /path/to/Proteus")
    probe(Path(sys.argv[1]).resolve())
