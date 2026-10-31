#!/usr/bin/env python3
"""Capture untouched background-gui frames around one click, without host input."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import json
import math
from pathlib import Path
import subprocess
import sys
import time


def command(args, output, name, operation, *parameters):
    argv = [sys.executable, str(args.helper), operation, args.session, *map(str, parameters)]
    receipt = {"name": name, "argv": argv, "log": str(output / (name + ".log")),
               "start_monotonic_ns": time.monotonic_ns(), "exit_status": None}
    try:
        with Path(receipt["log"]).open("wb") as log:
            result = subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT, timeout=15)
        receipt["exit_status"] = result.returncode
    except (OSError, subprocess.TimeoutExpired) as exc:
        receipt["error"] = type(exc).__name__ + ": " + str(exc)
    finally:
        receipt["end_monotonic_ns"] = time.monotonic_ns()
    return receipt


def capture(args):
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    receipt = {"status": "running", "session": args.session, "helper": str(args.helper),
               "requested_frames": args.frames, "interval_s": args.interval,
               "click_coordinates": args.click, "frames": [], "click": None,
               "timing_semantics": "monotonic command invocation/completion bounds; "
                                   "not exact display capture timestamps",
               "start_monotonic_ns": time.monotonic_ns()}
    click = None
    try:
        with ThreadPoolExecutor(max_workers=1) as executor:
            # Frame 0 is a settled baseline. The one click then runs alongside
            # the remaining screenshot loop; helper owns all private input.
            next_frame = time.monotonic()
            for index in range(args.frames):
                time.sleep(max(0, next_frame - time.monotonic()))
                path = args.output / f"frame-{index:03d}.png"
                frame = command(args, args.output, f"frame-{index:03d}",
                                "screenshot", "--path", path)
                frame.update(index=index, path=str(path), png_exists=path.is_file())
                receipt["frames"].append(frame)
                if frame["exit_status"] != 0 or not frame["png_exists"]:
                    raise RuntimeError(f"Screenshot {index} failed; see {frame['log']}")
                if index == 0:
                    click = executor.submit(command, args, args.output, "click", "click", *args.click)
                    next_frame = time.monotonic()
                next_frame += args.interval
            receipt["click"] = click.result()
            if receipt["click"]["exit_status"] != 0:
                raise RuntimeError("Click failed; see click.log")
        receipt["status"] = "commands-completed"
    except BaseException as exc:
        receipt.update(status="failed", error=type(exc).__name__ + ": " + str(exc))
    finally:
        if click is not None and receipt["click"] is None:
            try:
                receipt["click"] = click.result()
            except BaseException as exc:
                receipt["click"] = {"error": type(exc).__name__ + ": " + str(exc)}
        receipt["end_monotonic_ns"] = time.monotonic_ns()
        (args.output / "capture.json").write_text(json.dumps(receipt, ensure_ascii=False,
                                                           indent=2) + "\n")
    print(json.dumps({"status": receipt["status"], "output": str(args.output),
                      "frames": len(receipt["frames"])}), flush=True)
    return 0 if receipt["status"] == "commands-completed" else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--helper", type=Path, required=True, help="Absolute path to bgui.py")
    parser.add_argument("--session", required=True, help="Existing private background-gui session id")
    parser.add_argument("--output", type=Path, required=True, help="New directory; must not exist")
    parser.add_argument("--click", type=int, nargs=2, required=True, metavar=("X", "Y"))
    parser.add_argument("--frames", type=int, default=12, help="Includes baseline frame (default: 12)")
    parser.add_argument("--interval", type=float, default=.08, help="Requested command cadence in seconds")
    args = parser.parse_args()
    if not args.helper.is_absolute() or not args.helper.is_file():
        parser.error("--helper must be an existing absolute bgui.py path")
    args.helper = args.helper.resolve()
    args.output = args.output.resolve()
    if args.output.exists():
        parser.error("--output must not exist")
    if args.frames < 2:
        parser.error("--frames must be at least 2 (baseline and motion)")
    if not math.isfinite(args.interval) or args.interval <= 0:
        parser.error("--interval must be finite and positive")
    if min(args.click) < 0:
        parser.error("--click coordinates must be nonnegative")
    try:
        return capture(args)
    except OSError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
