#!/usr/bin/env python3
"""Drive the owned SVG fixture, verifying actual screenshot pixels per stage.

Requires the background-gui helper and Pillow for PNG inspection only. It
does not edit screenshots or interact with the user's display.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import time

from PIL import Image

HERE = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(HERE))
import corpus


def save(path, value):
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def colors(stage):
    return [tuple((stage * factor + index * offset + base) % 256
                  for factor, offset, base in ((63, 97, 40), (31, 71, 70), (17, 43, 100)))
            for index in range(2)]


def pixels(path, stage):
    with Image.open(path) as frame:
        counts = Counter(frame.convert("RGB").getdata())
    return [counts[color] for color in colors(stage)]


def state(output):
    # The runner may be replacing a large sample snapshot while we poll it.
    for attempt in range(10):
        try:
            value = json.loads((output / "state.json").read_text())
            break
        except json.JSONDecodeError:
            if attempt == 9:
                raise
            time.sleep(.02)
    if value["status"] != "running" or value["fixture_errors"]:
        raise RuntimeError(f"fixture not running cleanly: {value['status']}")
    return value


def wait_for(output, predicate):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        value = state(output)
        if predicate(value):
            return
        time.sleep(.1)
    raise TimeoutError("fixture stage callback not received")


def mark(output, name):
    (output / "control" / f"mark-{name}").touch(exist_ok=True)
    # A control filename is a request, not an acknowledgement. Advancing now
    # could sample the next source (or even teardown) under the previous label.
    wait_for(output, lambda value: name in value["marked_samples"])


def screenshot(args, path):
    subprocess.run([sys.executable, str(args.bgui), "screenshot", args.session,
                    "--path", str(path)], check=True, capture_output=True, text=True)


def capture(args, stage, empty=False):
    path = args.output / ("cleared.png" if empty else f"stage-{stage:02}.png")
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if path.exists():
            stem = "cleared" if empty else f"stage-{stage:02}"
            serial = 1
            while (args.output / f"{stem}-{serial:03}.png").exists():
                serial += 1
            path = args.output / f"{stem}-{serial:03}.png"
        screenshot(args, path)
        counts = pixels(path, stage)
        accepted = all(count == 0 for count in counts) if empty else min(counts) >= 50000
        if accepted:
            return {"stage": stage, "empty": empty, "pixel_counts": counts,
                    "expected_colors": colors(stage), "screenshot": path.name,
                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
        time.sleep(.2)
    raise RuntimeError(f"actual SVG pixels do not match stage {stage}: {counts}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--session", required=True)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--bgui", type=Path, default=Path.home() / ".codex/skills/background-gui/scripts/bgui.py")
    parser.add_argument("--settle", type=float, default=2.)
    parser.add_argument("--stop", action="store_true")
    args = parser.parse_args()
    args.output = args.output.resolve(strict=True)
    if not 0 <= args.settle <= 10:
        parser.error("--settle must be in 0..10 seconds")
    verification = args.output / "pixel-verification.json"
    if verification.exists():
        parser.error("pixel verification already exists; preserve the previous run")
    stage = state(args.output)["svg_stage"]
    receipt = {"kind": "actual native PNG pixel verification", "session": args.session,
               "helper_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
               "settle_seconds": args.settle, "initial_stage": stage, "captures": []}
    # A manually inspected initial stage may precede automation. Verify its
    # saved original PNG too; never infer paint from extension callbacks.
    for previous in range(stage):
        path = args.output / f"stage-{previous:02}.png"
        counts = pixels(path, previous)
        if min(counts) < 50000:
            raise RuntimeError(f"saved initial stage {previous} was not painted")
        receipt["captures"].append({"stage": previous, "empty": False,
                                    "pixel_counts": counts, "expected_colors": colors(previous),
                                    "screenshot": path.name,
                                    "sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
    for current in range(stage, corpus.STAGES):
        if current != stage:
            (args.output / "control" / f"next-{current:02}").touch(exist_ok=False)
        wait_for(args.output, lambda value: current in value["rendered_stages"])
        # First confirm actual paint; settling starts after that confirmation.
        capture(args, current)
        time.sleep(args.settle)
        receipt["captures"].append(capture(args, current))
        mark(args.output, f"stage-{current:02}")
        save(verification, receipt)
        print(f"Verified SVG stage {current:02}: {receipt['captures'][-1]['pixel_counts']}", flush=True)
    (args.output / "control/clear").touch(exist_ok=False)
    wait_for(args.output, lambda value: value["cleared"])
    capture(args, corpus.STAGES - 1, empty=True)
    time.sleep(args.settle)
    receipt["captures"].append(capture(args, corpus.STAGES - 1, empty=True))
    mark(args.output, "cleared")
    receipt["status"] = "passed"
    receipt["limits"] = "Two expected solid-color regions prove paint per stage, not all SVG geometry or GPU allocation release."
    save(verification, receipt)
    if args.stop:
        (args.output / "control/stop").touch()
    print("Verified cleared chat", flush=True)


if __name__ == "__main__":
    main()
