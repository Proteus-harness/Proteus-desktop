#!/usr/bin/env python3
"""Snapshot file/anonymous mappings of this fixture's still-running Native."""
import argparse
import hashlib
import json
from pathlib import Path
import re


KEYS = ("Rss", "Pss", "Private_Clean", "Private_Dirty", "Anonymous", "SwapPss")
HEADER = re.compile(r"^[0-9a-f]+-[0-9a-f]+\s+\S+\s+\S+\s+\S+\s+\d+\s*(.*)$")


def collect(run):
    state = json.loads((run / "state.json").read_text())
    if state["status"] != "running":
        raise ValueError("Capture without --stop, then collect before teardown")
    if not state["cleared"]:
        raise ValueError("Wait for the fixture's cleared capture before collecting mappings")
    proc = Path("/proc") / str(state["pid"])
    identity = (proc / "stat").read_text().rsplit(")", 1)[1].split()[19]
    exe = (proc / "exe").resolve(strict=True)
    if hashlib.sha256(exe.read_bytes()).hexdigest() != state["executable_sha256"]:
        raise ValueError("Native executable changed")
    totals, files = dict.fromkeys(KEYS, 0), {}
    for line in (proc / "smaps").read_text().splitlines():
        header = HEADER.match(line)
        if header:
            name = header[1] or "<anonymous>"
            if name == str(exe):
                name = "<native executable>"
            current = files.setdefault(name, dict.fromkeys(KEYS, 0))
        elif ":" in line:
            key, value = line.split(":", 1)
            if key in totals:
                amount = int(value.split()[0])
                current[key] += amount
                totals[key] += amount
    if (proc / "stat").read_text().rsplit(")", 1)[1].split()[19] != identity:
        raise ValueError("Native identity changed during capture")
    return {"pid": state["pid"], "start_ticks": identity, "elf_sha256": state["executable_sha256"],
            "helper_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "units": "KiB", "totals": totals,
            "files": dict(sorted(files.items(), key=lambda row: row[1]["Pss"], reverse=True)),
            "limits": "One post-clear /proc mapping snapshot; not a heap allocation profile or GPU VRAM measurement."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", type=Path, required=True)
    args = parser.parse_args()
    result = collect(args.run)
    with (args.run / "mappings-cleared.json").open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps(result["totals"]))


if __name__ == "__main__":
    main()
