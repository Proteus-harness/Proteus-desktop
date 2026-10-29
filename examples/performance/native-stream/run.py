#!/usr/bin/env python3
"""Launch a ready Native against a bounded synthetic loopback SSE transport."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import signal
import subprocess
import sys
import time

from fixture_server import Fixture, message
from scenario import BOOT, COPY_ROWS, EXPECTED, STAGES, emit_stage


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".new")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def launched_executable(pid):
    """Fingerprint the running image, after any same-PID wrapper exec."""
    proc_exe = Path(f"/proc/{pid}/exe")
    path = os.readlink(proc_exe)
    name = Path(path).name
    if re.fullmatch(r"(?:python(?:\d+(?:\.\d+)*)?|pypy\d*|node(?:-\d+)?|"
                    r"bash|dash|sh|zsh|fish|env|busybox|perl|ruby|lua(?:[\d.]+)?)", name):
        raise RuntimeError("Launched PID still belongs to an interpreter/launcher; "
                           "the wrapper must exec Native in its own PID")
    with proc_exe.open("rb") as stream:
        stat = os.fstat(stream.fileno())
        if stream.read(4) != b"\x7fELF":
            raise RuntimeError("Launched executable is not a native Linux ELF image")
        stream.seek(0)
        checksum = hashlib.file_digest(stream, "sha256").hexdigest()
    current = proc_exe.stat()
    if (os.readlink(proc_exe) != path or
            (current.st_dev, current.st_ino) != (stat.st_dev, stat.st_ino)):
        raise RuntimeError("Launched executable changed while recording provenance")
    return {"executable": path, "executable_sha256": checksum,
            "executable_verified_at_ns": time.monotonic_ns()}


def stop_native(process):
    # Attach mode with an empty extension directory launches no backend/worker.
    for sig in (signal.SIGTERM, signal.SIGKILL):
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=3)
            return
        except subprocess.TimeoutExpired:
            continue
    raise RuntimeError("Native process did not exit")


def advance(args):
    state = json.loads((args.output / "state.json").read_text())
    if state["status"] != "running":
        raise RuntimeError("Fixture is not running")
    if args.stage != "stop":
        previous = state["stage"]
        expected = STAGES[0] if previous == "ready" else (
            STAGES[STAGES.index(previous) + 1] if previous != STAGES[-1] else None)
        if args.stage != expected:
            raise RuntimeError(f"Next stage is {expected!r}, not {args.stage!r}")
    # Exclusive creation prevents a second operator from silently advancing twice.
    (args.output / "control" / args.stage).touch(exist_ok=False)
    print("Requested " + args.stage, flush=True)


def check_copy(args):
    state = json.loads((args.output / "state.json").read_text())
    if state["stage"] != args.stage or state["status"] != "running":
        raise RuntimeError("Copy must be checked while its stage is held live")
    if args.row not in COPY_ROWS.get(args.stage, ()):
        raise ValueError("Message Copy is hidden while this row is streaming")
    expected = EXPECTED[args.stage][args.row].encode()
    actual = args.file.read_bytes()
    passed = actual == expected
    save(args.output / f"copy-{args.stage}-{args.row}.json", {
        "passed": passed, "stage": args.stage, "row": args.row,
        "source": str(args.file.resolve()), "at_ns": time.monotonic_ns(),
        "expected_bytes": len(expected), "actual_bytes": len(actual),
        "actual_sha256": hashlib.sha256(actual).hexdigest(),
        "expected_sha256": hashlib.sha256(expected).hexdigest(),
    })
    if not passed:
        raise RuntimeError("Clipboard bytes differ; no whitespace normalization is applied")
    print(f"PASS {args.stage}/{args.row}: {len(actual)} exact UTF-8 bytes", flush=True)


def run(args):
    args.native = args.native.resolve(strict=True)
    if not os.access(args.native, os.X_OK):
        raise RuntimeError("--native must be an already-built executable")
    if not 10 <= args.timeout <= 1800:
        raise ValueError("--timeout must be between 10 and 1800 seconds")
    args.output = args.output.resolve()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for directory in ("control", "workspace", "session", "home", "runtime", "config",
                      "xdg-config", "xdg-data", "xdg-cache", "empty-extensions"):
        (args.output / directory).mkdir(mode=0o700)
    for stage, rows in EXPECTED.items():
        for row, text in rows.items():
            (args.output / f"expected-{stage}-{row}.txt").write_bytes(text.encode())
    fixture = Fixture(args.output / "workspace", args.output / "session",
                      secrets.token_hex(24), [message(BOOT, 1)]).start()
    process = None
    state = {"status": "starting", "stage": "ready", "origin": fixture.origin,
             "launcher": str(args.native), "launcher_sha256": digest(args.native),
             "source_sha256": {path.name: digest(path) for path in
                               Path(__file__).parent.glob("*.py")},
             "timeout_s": args.timeout, "events_written": 0}
    started = time.monotonic()
    interrupted = False

    def on_signal(_signum, _frame):
        nonlocal interrupted
        interrupted = True

    previous_handlers = {sig: signal.signal(sig, on_signal)
                         for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        env = os.environ.copy()
        env.update({key: str(args.output / directory) for key, directory in (
            ("HOME", "home"), ("PROTEUS_HOME", "runtime"),
            ("PROTEUS_CONFIG_HOME", "config"), ("XDG_CONFIG_HOME", "xdg-config"),
            ("XDG_DATA_HOME", "xdg-data"), ("XDG_CACHE_HOME", "xdg-cache"),
            ("PROTEUS_EXTENSIONS_DIR", "empty-extensions"))})
        env["PROTEUS_STARTUP_TRACE"] = "1"
        with (args.output / "native.log").open("w") as log:
            process = subprocess.Popen([str(args.native), str(args.output / "workspace"),
                                        "--origin", fixture.origin, "--token", fixture.token],
                                       env=env, stdout=log, stderr=log, start_new_session=True)
            state["pid"] = process.pid
            save(args.output / "state.json", state)
            next_stage = 0
            while not interrupted:
                if fixture.errors:
                    raise RuntimeError("; ".join(fixture.errors))
                if process.poll() is not None:
                    raise RuntimeError(f"Native exited early ({process.returncode}); see native.log")
                if time.monotonic() - started > args.timeout:
                    raise TimeoutError("Fixture deadline reached")
                if (args.output / "control/stop").exists():
                    break
                if fixture.connected.is_set() and state["status"] == "starting":
                    # A shebang/Python wrapper can exec a different Native at
                    # the same path/PID. Hash its actual kernel image at READY.
                    state.update(launched_executable(process.pid))
                    state["status"] = "running"
                    save(args.output / "state.json", state)
                    print("READY " + json.dumps(state), flush=True)
                if state["status"] == "running" and next_stage < len(STAGES):
                    stage = STAGES[next_stage]
                    if (args.output / "control" / stage).exists():
                        emit_stage(fixture, stage)
                        with fixture.lock:
                            last = next(row["trace_seq"] for row in reversed(fixture.trace)
                                        if row["kind"] == "event_enqueued")
                        until = time.monotonic() + 5
                        while True:
                            with fixture.lock:
                                written = any(row["kind"] == "event_written" and
                                              row["enqueued_trace_seq"] == last
                                              for row in fixture.trace)
                            if written:
                                break
                            if interrupted or fixture.errors or time.monotonic() > until:
                                raise TimeoutError("Stage SSE delivery did not finish")
                            time.sleep(.02)
                        state["stage"] = stage
                        state["stage_written_ns"] = time.monotonic_ns()
                        with fixture.lock:
                            state["events_written"] = sum(row["kind"] == "event_written"
                                                          for row in fixture.trace)
                            save(args.output / "trace.json", fixture.trace)
                        save(args.output / "state.json", state)
                        print("STAGE " + stage, flush=True)
                        next_stage += 1
                time.sleep(.05)
        state["status"] = "stopped"
    except BaseException as exc:
        state.update(status="failed", error=type(exc).__name__ + ": " + str(exc))
        raise
    finally:
        try:
            fixture.stopping.set()
            if process is not None:
                stop_native(process)
        finally:
            fixture.close()
            state["fixture_errors"] = fixture.errors
            state["elapsed_s"] = time.monotonic() - started
            with fixture.lock:
                save(args.output / "trace.json", fixture.trace)
            save(args.output / "state.json", state)
            for sig, handler in previous_handlers.items():
                signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    launch = commands.add_parser("run")
    launch.add_argument("--native", type=Path, required=True)
    launch.add_argument("--output", type=Path, required=True)
    launch.add_argument("--timeout", type=float, default=600)
    control = commands.add_parser("advance")
    control.add_argument("--output", type=Path, required=True)
    control.add_argument("--stage", choices=(*STAGES, "stop"), required=True)
    check = commands.add_parser("check-copy")
    check.add_argument("--output", type=Path, required=True)
    check.add_argument("--stage", choices=COPY_ROWS, required=True)
    check.add_argument("--row", choices=("primary", "secondary"), default="primary")
    check.add_argument("--file", type=Path, required=True)
    args = parser.parse_args()
    {"run": run, "advance": advance, "check-copy": check_copy}[args.command](args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
