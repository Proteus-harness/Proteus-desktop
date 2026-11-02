#!/usr/bin/env python3
"""Native Terminal lifecycle fixture with Settings; no Cargo/Core/models."""
import argparse
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import secrets
import shutil
import signal
import subprocess
import sys
import time

from fixture import TerminalFixture, TRANSPORT

HERE = Path(__file__).resolve().parent
STREAM = HERE.parent / "native-stream"
sys.path.insert(0, str(STREAM))
spec = importlib.util.spec_from_file_location("native_terminal_launch_helpers", STREAM / "run.py")
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
sys.path.pop(0)
save, digest = helpers.save, helpers.digest


def prepare(args):
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for name in ("control", "workspace", "session", "home", "runtime", "config",
                 "xdg-config", "xdg-data", "xdg-cache", "extensions"):
        (args.output / name).mkdir(mode=0o700)
    # Own the installed copy; toggles/removal never alter the source portable.
    shutil.copytree(args.terminal, args.output / "extensions/terminal")
    prefs = args.output / "xdg-config/dev.proteus.native"
    prefs.mkdir(mode=0o700)
    save(prefs / "preferences.json", {"motion": "off", "preferred_agent": "proteus"})
    fixture = TerminalFixture(args.output, secrets.token_hex(24))
    env = {key: str(args.output / name) for key, name in (
        ("HOME", "home"), ("PROTEUS_HOME", "runtime"), ("PROTEUS_CONFIG_HOME", "config"),
        ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data"),
        ("XDG_CACHE_HOME", "xdg-cache"), ("PROTEUS_EXTENSIONS_DIR", "extensions"))}
    env.update(SHELL="/bin/sh", PROTEUS_STARTUP_TRACE="1", PYTHONDONTWRITEBYTECODE="1")
    return fixture, env


def enable_subreaper():
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0):  # Linux PR_SET_CHILD_SUBREAPER
        raise OSError(ctypes.get_errno(), "Cannot enable fixture child subreaper")


def reap_adopted():
    """After Native exits, stop/reap only children adopted by this fixture."""
    reaped, deadline = [], time.monotonic() + 4
    while True:
        while True:
            try:
                pid, status = os.waitpid(-1, os.WNOHANG)
            except ChildProcessError:
                return reaped
            if not pid:
                break
            reaped.append({"pid": pid, "wait_status": status})
        children = Path(f"/proc/{os.getpid()}/task/{os.getpid()}/children").read_text().split()
        for value in children:
            try:
                os.kill(int(value), signal.SIGKILL if time.monotonic() > deadline - 2 else signal.SIGTERM)
            except ProcessLookupError:
                pass
        if time.monotonic() > deadline:
            raise RuntimeError("Owned terminal children did not reap")
        time.sleep(.02)


def run(args):
    enable_subreaper()
    fixture, env = prepare(args)
    process = None
    interrupted = False
    started = time.monotonic()
    state = {"status": "starting", "origin": fixture.origin,
             "launcher": str(args.native), "launcher_sha256": digest(args.native),
             "source_sha256": {path.name: digest(path) for path in HERE.glob("*.py")},
             "transport_sha256": digest(TRANSPORT),
             "launch_helpers_sha256": digest(STREAM / "run.py"),
             "terminal_files_sha256": {str(path.relative_to(args.terminal)): digest(path)
                 for path in args.terminal.rglob("*") if path.is_file()}}

    def record():
        with fixture.lock:
            state.update(fixture.metrics(), fixture_errors=list(fixture.errors))
            save(args.output / "trace.json", fixture.trace)
        save(args.output / "state.json", state)

    def on_signal(_sig, _frame):
        nonlocal interrupted
        interrupted = True

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        fixture.start()
        argv = [str(args.native), fixture.workspace, "--origin", fixture.origin, "--token", fixture.token]
        save(args.output / "launch.json", {"argv": argv[:-1] + ["<redacted>"], "env": env})
        with (args.output / "native.log").open("w") as log:
            process = subprocess.Popen(argv, env={**os.environ, **env}, cwd=fixture.workspace,
                                       stdout=log, stderr=log, start_new_session=True)
        state["pid"] = process.pid
        record()
        print("STARTED " + json.dumps({"pid": process.pid, "output": str(args.output)}), flush=True)
        while not interrupted and not (args.output / "control/stop").exists():
            if fixture.errors:
                raise RuntimeError("; ".join(fixture.errors))
            if process.poll() is not None:
                state["native_exit"] = process.returncode
                raise RuntimeError(f"Native exited without a stop request ({process.returncode})")
            if time.monotonic() - started > args.timeout:
                raise TimeoutError("Terminal fixture deadline reached")
            if fixture.connected.is_set() and state["status"] == "starting":
                state.update(helpers.launched_executable(process.pid), status="running")
                print("READY · synthetic session + Settings; GUI acceptance is separate", flush=True)
            record()
            time.sleep(.1)
        state["status"] = "stopped"
    except BaseException as exc:
        state.update(status="failed", error=f"{type(exc).__name__}: {exc}")
        raise
    finally:
        fixture.stopping.set()
        try:
            if process:
                helpers.stop_native(process)
        finally:
            try:
                state["reaped_children"] = reap_adopted()
            finally:
                try:
                    fixture.close()
                finally:
                    state["elapsed_s"] = time.monotonic() - started
                    record()
                    save(args.output / "report.json", state)
                    for sig, handler in previous.items():
                        signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native", type=Path, required=True, help="Ready Native executable")
    parser.add_argument("--terminal", type=Path, required=True, help="Ready Terminal package directory")
    parser.add_argument("--timeout", type=float, default=1800)
    args = parser.parse_args()
    args.native = args.native.resolve(strict=True)
    args.terminal = args.terminal.resolve(strict=True)
    args.output = args.output.resolve()
    if not args.native.is_file() or not os.access(args.native, os.X_OK):
        parser.error("--native must be an already-built executable")
    if not args.terminal.is_dir():
        parser.error("--terminal must be a ready package directory")
    manifest = json.loads((args.terminal / "extension.json").read_text())
    if manifest.get("id") != "terminal":
        parser.error("--terminal must identify the ready Terminal extension")
    if not args.output.is_relative_to((HERE.parents[2] / "build/verification").resolve()):
        parser.error("--output must be under Desktop build/verification")
    if not 10 <= args.timeout <= 7200:
        parser.error("--timeout must be between 10 and 7200 seconds")
    run(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
