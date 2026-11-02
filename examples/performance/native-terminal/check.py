#!/usr/bin/env python3
"""Record/check the fixture-owned PTY shell identity; never signals a process."""
import argparse
import json
from pathlib import Path
import re
import time


def identity(pid):
    """Linux start ticks fence PID reuse; /proc stat's comm may contain spaces."""
    try:
        text = Path(f"/proc/{pid}/stat").read_text()
    except FileNotFoundError:
        return None
    fields = text[text.rfind(")") + 2:].split()
    return {"pid": pid, "state": fields[0], "ppid": int(fields[1]), "start_ticks": int(fields[19])}


def owned_by(pid, native):
    visited = set()
    while pid > 1 and pid not in visited:
        if pid == native:
            return True
        visited.add(pid)
        row = identity(pid)
        if row is None:
            return False
        pid = row["ppid"]
    return False


def check(output, label, expected, *, remember=False):
    if not re.fullmatch(r"[a-zA-Z0-9_-]{1,64}", label):
        raise ValueError("Invalid proof label")
    state = json.loads((output / "state.json").read_text())
    if state["status"] != "running":
        raise RuntimeError("Native fixture must be running")
    shell_file = output / "control/shell-pid"
    remembered = output / f"shell-{label}.json"
    if remember:
        pid = int(shell_file.read_text().strip())
        row = identity(pid)
        if row is None or row["state"] == "Z" or not owned_by(pid, state["pid"]):
            raise RuntimeError("Shell is not a live descendant of the fixture Native")
        with remembered.open("x") as stream:
            json.dump(row, stream, indent=2)
    else:
        row = json.loads(remembered.read_text())
    current = identity(row["pid"])
    alive = (current is not None and current["state"] != "Z"
             and current["start_ticks"] == row["start_ticks"])
    passed = alive == (expected == "alive")
    if alive and not owned_by(row["pid"], state["pid"]):
        passed = False
    result = {"passed": passed, "expected": expected, "identity": row, "current": current,
              "native_pid": state["pid"], "at_ns": time.monotonic_ns()}
    path = output / f"proof-{label}-{expected}-{result['at_ns']}.json"
    path.write_text(json.dumps(result, indent=2) + "\n")
    if not passed:
        raise RuntimeError(f"Expected remembered shell {row['pid']} {expected}; see {path}")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--label", required=True, help="Remembered shell identity name")
    parser.add_argument("--expect", choices=("alive", "dead"), required=True)
    parser.add_argument("--remember", action="store_true", help="Read control/shell-pid exactly once")
    args = parser.parse_args()
    if args.remember and args.expect != "alive":
        parser.error("Remember a live shell before testing its lifecycle")
    print(json.dumps(check(args.output.resolve(), args.label, args.expect, remember=args.remember)))


if __name__ == "__main__":
    main()
