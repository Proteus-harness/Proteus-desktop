#!/usr/bin/env python3
"""Hold a synthetic sidebar tree; assert browsing never opens child sessions."""
import argparse
import copy
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time
import uuid
from urllib.request import ProxyHandler, Request, build_opener

from fixture_server import Fixture, SESSION_ID, message, require
from run import digest, launched_executable, save, stop_native

DIRECTORIES = ("control", "workspace", "other-project", "session", "home", "runtime",
               "config", "xdg-config", "xdg-data", "xdg-cache", "empty-extensions")


def session_id(number):
    return str(uuid.UUID(int=number))


class SidebarFixture(Fixture):
    """Summary-only descendants. Only MAIN is resumable, intentionally.

    Opening a child is outside this oracle: it fails the same strict transport
    used by all native-stream fixtures rather than silently serving MAIN data.
    """
    def __init__(self, output, token):
        super().__init__(output / "workspace", output / "session", token,
                         [message("SIDEBAR MAIN · этот чат уже существует.\n"
                                  "Раскрытие и поиск потомков не должны загружать их историю.", 100)])
        idle = self.activity()

        def summary(number, title, parent=None, other=False):
            return {"session_dir": self.session if number == 1 else str(output / f"session-{number}"),
                    "session_id": session_id(number), "parent_session_id": parent,
                    "workspace_path": str(output / "other-project") if other else self.workspace,
                    "message_count": 2,
                    "updated_at_ms": 1793059200000 + (100000 if number == 1 else number * 1000),
                    "preview": title, "activity": copy.deepcopy(idle)}

        self.catalog = [
            summary(1, "SIDEBAR MAIN · основной чат"),
            summary(2, "NEEDLE CHILD · исследование", SESSION_ID),
            summary(3, "NESTED CHILD · проверка", session_id(2)),
            summary(4, "ORPHAN · родитель недоступен", session_id(999)),
            summary(5, "OTHER PROJECT · основной чат", other=True),
            summary(6, "SECOND MAIN · тот же проект"),
        ]

    def session_summaries(self):
        return copy.deepcopy(self.catalog)


def assert_lazy(requests, session, errors=()):
    """A catalog can be read repeatedly; only the selected MAIN may be opened."""
    require(not errors, "Fixture rejected a request: " + "; ".join(errors))
    for row in requests:
        require(row["path"] in ("/bootstrap", "/sessions", "/events", "/config",
                                "/config/local", "/context", "/resume"),
                "Browsing created or mutated a session: " + row["path"])
        if row.get("session") is not None:
            require(row["session"] == session, "Browsing subscribed to another session")
        if row["path"] == "/resume":
            require(row.get("body", {}).get("session_dir") == session,
                    "Browsing resumed another session")
    return {"passed": True, "request_count": len(requests),
            "resumes": sum(row["path"] == "/resume" for row in requests),
            "subscriptions": sum(row["path"] == "/events" for row in requests),
            "catalog_reads": sum(row["path"] == "/sessions" for row in requests)}


def record(output, fixture, state):
    with fixture.lock:
        state["fixture_errors"] = list(fixture.errors)
        state["audit"] = assert_lazy(fixture.requests, fixture.session, fixture.errors)
        save(output / "trace.json", fixture.trace)
    save(output / "state.json", state)


def check_fixture(output):
    fixture = SidebarFixture(output, "synthetic-sidebar-smoke-token").start()
    try:
        client = build_opener(ProxyHandler({}))

        def request(path, body=None):
            req = Request(fixture.origin + path,
                          data=None if body is None else json.dumps(body).encode(),
                          headers={"Authorization": "Bearer " + fixture.token,
                                   "Content-Type": "application/json"})
            with client.open(req, timeout=3) as response:
                return json.load(response)

        catalog = request("/sessions")
        require(len(catalog) == 6, "Catalog changed")
        require(catalog[1]["parent_session_id"] == SESSION_ID, "Missing direct child")
        require(catalog[2]["parent_session_id"] == catalog[1]["session_id"], "Missing nested child")
        require(catalog[3]["parent_session_id"] not in {row["session_id"] for row in catalog},
                "Missing orphan case")
        require(catalog[4]["workspace_path"] != fixture.workspace, "Missing second project")
        require(request("/bootstrap")["session_dir"] == fixture.session, "Wrong initial session")
        request("/resume", {"id": "sidebar-smoke", "session_dir": fixture.session})
        record(output, fixture, {"status": "fixture-smoke-passed", "native_verified": False,
                                 "session": fixture.session})
        print("PASS fixture: 6 summaries, nested child, orphan, two projects; no GUI checked")
    finally:
        fixture.close()


def hold(args):
    fixture = SidebarFixture(args.output, secrets.token_hex(24)).start()
    process = None
    started = time.monotonic()
    interrupted = False
    env = {key: str(args.output / directory) for key, directory in (
        ("HOME", "home"), ("PROTEUS_HOME", "runtime"), ("PROTEUS_CONFIG_HOME", "config"),
        ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data"),
        ("XDG_CACHE_HOME", "xdg-cache"), ("PROTEUS_EXTENSIONS_DIR", "empty-extensions"))}
    argv = ([str(args.native), fixture.workspace, "--origin", fixture.origin,
             "--token", fixture.token] if args.native else None)
    save(args.output / "launch.json", {"argv": argv, "env": env, "origin": fixture.origin,
                                      "workspace": fixture.workspace,
                                      "token": fixture.token})
    state = {"status": "serving", "session": fixture.session, "origin": fixture.origin,
             "native_verified": False, "checks": 0,
             "source_sha256": {name: digest(Path(__file__).with_name(name)) for name in
                               ("sidebar_demo.py", "fixture_server.py", "run.py")}}

    def on_signal(_signum, _frame):
        nonlocal interrupted
        interrupted = True

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        if argv:
            with (args.output / "native.log").open("w") as log:
                process = subprocess.Popen(argv, env={**os.environ, **env}, stdout=log, stderr=log,
                                           start_new_session=True)
            state.update(pid=process.pid, launcher=str(args.native),
                         launcher_sha256=digest(args.native))
        print("SERVING " + json.dumps({"origin": fixture.origin, "launch": str(args.output / "launch.json"),
                                      "instructions": "Expand MAIN, expand CHILD, search NEEDLE, clear search, "
                                                      "open current project; never click a child row."}), flush=True)
        while not interrupted and not (args.output / "control/stop").exists():
            if process and process.poll() is not None:
                raise RuntimeError(f"Native exited early ({process.returncode}); see native.log")
            if time.monotonic() - started > args.timeout:
                raise TimeoutError("Sidebar demo deadline reached")
            if fixture.connected.is_set() and state["status"] == "serving":
                if process:
                    state.update(launched_executable(process.pid))
                state["status"] = "connected"
                print("READY · MAIN SSE connected; check the native window separately", flush=True)
            control = args.output / "control/check"
            if control.exists():
                control.unlink()
                state["checks"] += 1
                print("CHECK " + json.dumps(assert_lazy(fixture.requests, fixture.session,
                                                        fixture.errors)), flush=True)
            record(args.output, fixture, state)
            time.sleep(.25)
        state["status"] = "stopped"
    except BaseException as exc:
        state.update(status="failed", error=type(exc).__name__ + ": " + str(exc))
        raise
    finally:
        fixture.stopping.set()
        try:
            if process:
                stop_native(process)
        finally:
            fixture.close()
            # Failed requests remain reviewable even when the audit rejects them.
            save(args.output / "trace.json", fixture.trace)
            state.update(fixture_errors=fixture.errors, elapsed_s=time.monotonic() - started)
            save(args.output / "state.json", state)
            for sig, handler in previous.items():
                signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, help="Already built Native; omitted for server-only hold")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--check-fixture", action="store_true")
    args = parser.parse_args()
    if not 10 <= args.timeout <= 1800:
        parser.error("--timeout must be between 10 and 1800 seconds")
    if args.native:
        args.native = args.native.resolve(strict=True)
        if not os.access(args.native, os.X_OK):
            parser.error("--native must be executable")
    args.output = args.output.resolve()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for directory in DIRECTORIES:
        (args.output / directory).mkdir(mode=0o700)
    check_fixture(args.output) if args.check_fixture else hold(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError, AssertionError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
