#!/usr/bin/env python3
"""Hold a ready Native in real thinking state for scroll/hover visual checks."""
import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

from fixture_server import Fixture, SESSION_ID, THREAD_ID, TURN_ID, message
from run import digest, launched_executable, save, stop_native

DIRECTORIES = ("control", "workspace", "session", "home", "runtime", "config",
               "xdg-config", "xdg-data", "xdg-cache", "empty-extensions")


def transcript():
    rows = []
    for index in range(20):
        role = "user" if index % 2 == 0 else "assistant"
        rows.append(message(
            f"STATUS DEMO · сообщение {index + 1}\n"
            "Это синтетический текст для прокрутки длинного чата.\n"
            "Колесо вверх скрывает конец; кнопка вниз возвращает к нему.",
            1000 + index, role=role))
    rows.append(message("Подожди с ответом: проверяю «Думаю», прокрутку и кнопку вниз.",
                        1020, role="user"))
    for index, row in enumerate(rows):
        row["timestamp_ms"] = 1793059200000 + index * 60000
    return rows


def begin(fixture):
    fixture.run_id = "run-synthetic-native-status"
    fixture.execution("running")
    fixture.runtime("TurnStarted", {"session_id": SESSION_ID,
                                   "thread_id": THREAD_ID, "turn_id": TURN_ID})


def finish(fixture):
    # No assistant delta or replacement snapshot: the status disappears from
    # the actual execution update, not from a synthetic answer hiding it.
    fixture.execution("success", terminal=True)


def delivered(fixture):
    with fixture.lock:
        last = next(row["trace_seq"] for row in reversed(fixture.trace)
                    if row["kind"] == "event_enqueued")
    until = time.monotonic() + 5
    while time.monotonic() < until:
        with fixture.lock:
            if any(row["kind"] == "event_written" and row["enqueued_trace_seq"] == last
                   for row in fixture.trace):
                return
        if fixture.errors:
            raise RuntimeError("; ".join(fixture.errors))
        time.sleep(.02)
    raise TimeoutError("Status SSE delivery did not finish")


def record(output, fixture, state):
    with fixture.lock:
        state["events_written"] = sum(row["kind"] == "event_written" for row in fixture.trace)
        save(output / "trace.json", fixture.trace)
    save(output / "state.json", state)


def check_fixture(output):
    """Permanent wire smoke; it deliberately makes no native GUI claim."""
    fixture = Fixture(output / "workspace", output / "session",
                      secrets.token_hex(24), transcript()).start()
    try:
        client = build_opener(ProxyHandler({}))
        request = Request(fixture.origin + "/events?" + urlencode({"session_dir": fixture.session}),
                          headers={"Authorization": "Bearer " + fixture.token})
        with client.open(request, timeout=3) as stream:
            def read():
                while True:
                    line = stream.readline()
                    if not line:
                        raise RuntimeError("Fixture SSE closed early")
                    if line.startswith(b"data: "):
                        return json.loads(line[6:])["event"]

            snapshot = read()
            assert len(snapshot["snapshot"]["transcript"]) == 21
            assert snapshot["snapshot"]["transcript"][-1]["role"] == "user"
            begin(fixture)
            running, turn = read(), read()
            assert running["execution"]["active"]["status"] == "running"
            assert turn["envelope"]["event"]["TurnStarted"]["thread_id"] == THREAD_ID
            finish(fixture)
            done = read()
            assert done["execution"]["active"] is None
            assert done["execution"]["last"]["status"] == "success"
            delivered(fixture)
            fixture.stopping.set()
        record(output, fixture, {"status": "fixture-smoke-passed", "rows": 21,
                                 "native_verified": False})
        print("PASS fixture: 21 rows, running, TurnStarted, success; no GUI checked", flush=True)
    finally:
        fixture.close()


def run(args):
    fixture = Fixture(args.output / "workspace", args.output / "session",
                      secrets.token_hex(24), transcript()).start()
    process = None
    started = time.monotonic()
    interrupted = False
    state = {"status": "starting", "stage": "connecting", "rows": 21,
             "origin": fixture.origin, "timeout_s": args.timeout,
             "launcher": str(args.native), "launcher_sha256": digest(args.native),
             "extensions": str(args.extensions) if args.extensions else None,
             "source_sha256": {name: digest(Path(__file__).with_name(name)) for name in
                               ("status_demo.py", "fixture_server.py", "run.py")}}

    def on_signal(_signum, _frame):
        nonlocal interrupted
        interrupted = True

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        env = os.environ.copy()
        env.update({key: str(args.output / directory) for key, directory in (
            ("HOME", "home"), ("PROTEUS_HOME", "runtime"), ("PROTEUS_CONFIG_HOME", "config"),
            ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data"),
            ("XDG_CACHE_HOME", "xdg-cache"), ("PROTEUS_EXTENSIONS_DIR", "empty-extensions"))})
        if args.extensions is not None:
            env["PROTEUS_EXTENSIONS_DIR"] = str(args.extensions)
        with (args.output / "native.log").open("w") as log:
            process = subprocess.Popen([str(args.native), str(args.output / "workspace"),
                                        "--origin", fixture.origin, "--token", fixture.token],
                                       env=env, stdout=log, stderr=log, start_new_session=True)
            state["pid"] = process.pid
            record(args.output, fixture, state)
            while not interrupted:
                if fixture.errors:
                    raise RuntimeError("; ".join(fixture.errors))
                if process.poll() is not None:
                    raise RuntimeError(f"Native exited early ({process.returncode}); see native.log")
                if time.monotonic() - started > args.timeout:
                    raise TimeoutError("Status demo deadline reached")
                if (args.output / "control/stop").exists():
                    break
                if fixture.connected.is_set() and state["status"] == "starting":
                    state.update(launched_executable(process.pid))
                    begin(fixture)
                    delivered(fixture)
                    state.update(status="running", stage="thinking")
                    record(args.output, fixture, state)
                    print("READY " + json.dumps(state), flush=True)
                if state["stage"] == "thinking" and (args.output / "control/finish").exists():
                    finish(fixture)
                    delivered(fixture)
                    state["stage"] = "finished"
                    record(args.output, fixture, state)
                    print("STAGE finished", flush=True)
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
            try:
                fixture.close()
            finally:
                state.update(fixture_errors=fixture.errors, elapsed_s=time.monotonic() - started)
                record(args.output, fixture, state)
                for sig, handler in previous.items():
                    signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path)
    parser.add_argument("--extensions", type=Path,
                        help="Optional selected ready-renderer directory; defaults to no extensions")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--check-fixture", action="store_true")
    args = parser.parse_args()
    if not 10 <= args.timeout <= 600:
        parser.error("--timeout must be between 10 and 600 seconds")
    if not args.check_fixture:
        if args.native is None:
            parser.error("--native READY is required")
        args.native = args.native.resolve(strict=True)
        if not os.access(args.native, os.X_OK):
            parser.error("--native must be an already-built executable")
    if args.extensions is not None:
        args.extensions = args.extensions.resolve(strict=True)
        if not args.extensions.is_dir():
            parser.error("--extensions must be an already prepared extension directory")
    args.output = args.output.resolve()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for directory in DIRECTORIES:
        (args.output / directory).mkdir(mode=0o700)
    if args.check_fixture:
        check_fixture(args.output)
    else:
        run(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError, AssertionError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
