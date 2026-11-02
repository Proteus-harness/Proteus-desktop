#!/usr/bin/env python3
"""Hold a multi-chat loopback fixture and optionally an already-built Native."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time

from fixture import ChatSpacesFixture


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".new")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")
    temporary.replace(path)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def layout(chat=None):
    return {"root": {"pane": {"id": 0, "tabs": [chat] if chat else [],
                              "pinned": 0, "active": chat}},
            "focused": 0, "last_chat": chat, "next_id": 1}


def prepare(output, native=None, extensions=None):
    """Only synthetic state is read/written; an attached origin is never persisted."""
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for name in ("control", "workspace", "child-workspace", "sessions", "home",
                 "runtime", "config", "xdg-config", "xdg-data", "xdg-cache"):
        (output / name).mkdir(mode=0o700)
    (output / "workspace/main.txt").write_text("ROOT A/B FILE · папка основного чата\n")
    (output / "child-workspace/child.txt").write_text("CHILD FILE · отдельная папка потомка\n")
    fixture = ChatSpacesFixture(output, secrets.token_hex(24))
    root = "chat:" + fixture.main.session
    preferences = {"preferred_agent": "proteus", "motion": "off",
                   "tabs": {"active": root, "roots": {root: layout(root)}, "loose": layout()},
                   "tabs_workspace": None}
    prefs_dir = output / "xdg-config/dev.proteus.native"
    prefs_dir.mkdir(mode=0o700)
    save(prefs_dir / "preferences.json", preferences)
    env = {key: str(output / name) for key, name in (
        ("HOME", "home"), ("PROTEUS_HOME", "runtime"), ("PROTEUS_CONFIG_HOME", "config"),
        ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data"),
        ("XDG_CACHE_HOME", "xdg-cache"))}
    env.update(SHELL="/bin/sh", PROTEUS_STARTUP_TRACE="1")
    if extensions:
        env["PROTEUS_EXTENSIONS_DIR"] = str(extensions)
    argv = ([str(native), fixture.main.workspace, "--origin", fixture.origin,
             "--token", fixture.token] if native else None)
    launch = {"argv": argv, "env": env, "origin": fixture.origin,
              "token": fixture.token, "workspace": fixture.main.workspace,
              "sessions": fixture.session_summaries(),
              "attached_scope_persisted": False,
              "extensions": str(extensions) if extensions else "launcher default"}
    save(output / "launch.json", launch)
    return fixture, launch


def stop_native(process):
    # Only this runner's private process group; never host application PIDs.
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
    raise RuntimeError("Native did not stop")


def hold(args):
    fixture, launch = prepare(args.output, args.native, args.extensions)
    process = None
    interrupted = False
    started = time.monotonic()
    state = {"status": "serving", "native_verified": False, "origin": fixture.origin,
             "source_sha256": {p.name: digest(p) for p in Path(__file__).parent.glob("*.py")},
             "transport_sha256": digest(Path(__file__).parent.parent / "native-stream/fixture_server.py")}

    def on_signal(_signum, _frame):
        nonlocal interrupted
        interrupted = True

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    fixture.start()
    try:
        if args.native:
            with (args.output / "native.log").open("w") as log:
                process = subprocess.Popen(launch["argv"], env={**os.environ, **launch["env"]},
                                           cwd=fixture.main.workspace, stdout=log, stderr=log,
                                           start_new_session=True)
            state.update(pid=process.pid, launcher=str(args.native),
                         launcher_sha256=digest(args.native))
        print("SERVING " + json.dumps({"output": str(args.output), "origin": fixture.origin,
                                       "launch": str(args.output / "launch.json")}), flush=True)
        while not interrupted and not (args.output / "control/stop").exists():
            if fixture.errors:
                raise RuntimeError("; ".join(fixture.errors))
            if process and process.poll() is not None:
                state.update(status="native-exited", native_exit=process.returncode)
                break
            if time.monotonic() - started > args.timeout:
                raise TimeoutError("Fixture deadline reached")
            if fixture.connected.is_set() and state["status"] == "serving":
                state["status"] = "connected"
                print("READY · SSE connected; native appearance still requires GUI verification", flush=True)
            with fixture.lock:
                state.update(request_count=len(fixture.requests),
                             fixture_errors=list(fixture.errors),
                             sessions=fixture.session_summaries())
                save(args.output / "trace.json", fixture.trace)
            save(args.output / "state.json", state)
            time.sleep(.25)
        if state["status"] != "native-exited":
            state["status"] = "stopped"
    except BaseException as exc:
        state.update(status="failed", error=f"{type(exc).__name__}: {exc}")
        raise
    finally:
        fixture.stopping.set()
        try:
            if process:
                stop_native(process)
        finally:
            fixture.close()
            save(args.output / "trace.json", fixture.trace)
            state.update(fixture_errors=fixture.errors, elapsed_s=time.monotonic() - started)
            save(args.output / "state.json", state)
            for sig, handler in previous.items():
                signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--native", type=Path, help="Absolute path to ready Native or portable launcher")
    mode.add_argument("--serve-only", action="store_true", help="Write launch.json and hold only transport")
    parser.add_argument("--extensions", type=Path, help="Ready portable share/extensions for a debug Native")
    parser.add_argument("--timeout", type=float, default=1200)
    args = parser.parse_args()
    if not 10 <= args.timeout <= 7200:
        parser.error("--timeout must be between 10 and 7200 seconds")
    if args.native:
        if not args.native.is_absolute():
            parser.error("--native must be absolute")
        args.native = args.native.resolve(strict=True)
        if not os.access(args.native, os.X_OK):
            parser.error("--native must be executable")
    if args.extensions:
        args.extensions = args.extensions.resolve(strict=True)
        if not args.extensions.is_dir():
            parser.error("--extensions must be a directory")
    args.output = args.output.resolve()
    hold(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
