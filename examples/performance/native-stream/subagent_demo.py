#!/usr/bin/env python3
"""Synthetic readonly subagent panes: exact identities, lazy pages, no child runtime."""
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
from urllib.parse import parse_qs, urlencode, urlsplit
from urllib.request import ProxyHandler, Request, build_opener

from fixture_server import Fixture, SESSION_ID, message, require
from run import digest, launched_executable, save, stop_native

DIRECTORIES = ("control", "workspace", "session", "child-A", "child-B", "home", "runtime",
               "config", "xdg-config", "xdg-data", "xdg-cache", "empty-extensions")
CHILD_IDS = {"A": "00000000-0000-0000-0000-0000000000a1",
             "B": "00000000-0000-0000-0000-0000000000b1"}
THREAD_IDS = {"A": "00000000-0000-0000-0000-0000000000a2",
              "B": "00000000-0000-0000-0000-0000000000b2"}
ROLE = "explorer"


def transcript_message(text, number, role="assistant"):
    row = message(text, number, role)
    row["timestamp_ms"] = 1793059200000 + number
    return row


class SubagentFixture(Fixture):
    def __init__(self, output, token, *, deferred_a=False):
        output = output.resolve()
        self.children = {}
        for key in ("A", "B"):
            self.children[key] = {
                "session": {"session_id": CHILD_IDS[key], "session_dir": str(output / f"child-{key}")},
                "revision": f"child-{key}-revision-1", "base": None,
                "messages": [transcript_message(f"CHILD {key} ONLY · row {i:03d}\n"
                                                f"Readonly saved {key} conversation.",
                                                (10000 if key == "A" else 20000) + i,
                                                "user" if i % 2 == 0 else "assistant")
                             for i in range(150)]}
        live = transcript_message("", 201, "system")
        live["subagent"] = {"child_thread_id": THREAD_IDS["A"],
                            "session": copy.deepcopy(self.children["A"]["session"]),
                            "role": ROLE, "description": "CHILD A · live saved conversation",
                            "status": "running", "iterations": None, "tools": []}
        completed = transcript_message("", 202, "tool")
        completed["tool"] = {
            "call_id": "completed-child-B-task", "name": "task",
            "args": {"agent_type": ROLE, "description": "CHILD B · completed saved conversation",
                     "prompt": "Synthetic task B; no model or command runs."},
            "effective_args": None, "status": "completed", "result": "Synthetic B task finished.",
            "metadata": {"tool": "task", "status": "completed",
                         "child_thread_id": THREAD_IDS["B"],
                         "subagent_session": copy.deepcopy(self.children["B"]["session"])}}
        unbound = transcript_message("", 203, "system")
        unbound["subagent"] = {"child_thread_id": "legacy-unbound-synthetic-thread", "session": None,
                               "role": ROLE, "description": "UNBOUND · no saved child session",
                               "status": "completed", "iterations": 1, "tools": []}
        super().__init__(output / "workspace", output / "session", token,
                         [transcript_message("SUBAGENT MAIN · Open A and B from the compact strips.\n"
                                             "Both use the same role; their histories must stay distinct.", 200),
                          live, completed, unbound])
        self.finished_a = False
        self.bound_a = not deferred_a
        if deferred_a:
            self.transcript[1]["subagent"]["session"] = None

    def session_summaries(self):
        rows = super().session_summaries()
        rows[0].update(preview="SUBAGENT MAIN", parent_session_id=None)
        for key, child in self.children.items():
            if key == "A" and not self.bound_a:
                continue
            active = key == "A" and not self.finished_a
            rows.append({**child["session"], "parent_session_id": SESSION_ID,
                         "workspace_path": self.workspace, "message_count": len(child["messages"]),
                         "updated_at_ms": 1793059200000, "preview": f"CHILD {key} · {ROLE}",
                         "activity": {"status": "running" if active else "idle",
                                      "running_runs": int(active), "running_run_ids": [],
                                      "pending_approvals": 0, "pending_user_inputs": 0}})
        return rows

    def bind_a(self):
        """Publish a new child's actual session while its task is still running."""
        with self.lock:
            require(not self.bound_a, "bindA already applied")
            self.bound_a = True
            session = self.children["A"]["session"]
            self.transcript[1]["subagent"]["session"] = copy.deepcopy(session)
            self.runtime("SubagentSessionBound", {"child_thread_id": THREAD_IDS["A"], **session})
            self.record("control_applied", control="bindA")

    def append_a(self):
        """Write a child reply without a relayed parent event, as the real peer does."""
        with self.lock:
            require(self.bound_a and not self.finished_a, "appendA requires a bound running child")
            child = self.children["A"]
            number = len(child["messages"])
            child["base"] = {"revision": child["revision"], "common_prefix": number}
            child["revision"] = f"child-A-live-{number + 1}"
            child["messages"].append(transcript_message("CHILD A LIVE · reply before completion", 10000 + number))
            self.record("control_applied", control="appendA", child_total=len(child["messages"]))

    def handle(self, h, method):
        parsed = urlsplit(h.path)
        if parsed.path != "/history/page":
            # Base explicitly rejects any child resume/subscription. Preserve all
            # parent transport checks rather than opening a second fake runtime.
            return super().handle(h, method)
        h.connection.settimeout(15)
        require(h.headers.get("Authorization") == "Bearer " + self.token, "Invalid fixture token")
        require(method == "GET" and not parsed.fragment, "History only supports GET without fragment")
        query = parse_qs(parsed.query, strict_parsing=True, keep_blank_values=True)
        require(set(query) <= {"session_dir", "before", "revision"}, "Unexpected history query key")
        require(all(len(values) == 1 and values[0] for values in query.values()),
                "History query values must be single and nonempty")
        directory = query.get("session_dir", [None])[0]
        key = next((key for key, child in self.children.items()
                    if child["session"]["session_dir"] == directory), None)
        require(key is not None, "History target must be an exact bound child directory")
        with self.lock:
            require(key != "A" or self.bound_a, "Child A is not bound yet")
            child = self.children[key]
            before = query.get("before", [None])[0]
            require(before is None or before.isascii() and before.isdigit(), "Invalid before position")
            require(before is None or "revision" in query, "Older page requires its display revision")
            revision = query.get("revision", [None])[0]
            require(revision is None or revision == child["revision"], "Stale or cross-child revision")
            end = len(child["messages"]) if before is None else int(before)
            require(0 <= end <= len(child["messages"]), "Page position outside child transcript")
            start = max(0, end - 50)
            row = self.record("http_request", method=method, path=parsed.path, session=directory,
                              child=key, before=None if before is None else end,
                              revision=child["revision"], body=None)
            require(len(self.requests) < 1024, "Fixture request count cap exceeded")
            self.requests.append(row)
            page = {"revision": child["revision"], "start": start, "total": len(child["messages"]),
                    "messages": copy.deepcopy(child["messages"][start:end]),
                    "base": copy.deepcopy(child["base"])}
        h.reply(200, page)

    def finish_a(self):
        """One durable append plus parent notification; never change parent session."""
        with self.lock:
            require(self.bound_a, "finishA requires a bound child")
            require(not self.finished_a, "finishA already applied")
            child = self.children["A"]
            child["base"] = {"revision": child["revision"], "common_prefix": len(child["messages"])}
            child["revision"] = "child-A-revision-2"
            child["messages"].append(transcript_message("CHILD A FINAL · final saved assistant answer", 10000 + len(child["messages"])))
            self.finished_a = True
            self.transcript[1]["subagent"].update(status="completed", iterations=4)
            self.runtime("SubagentFinished", {"role": ROLE, "status": "completed", "iterations": 4,
                                               "child_thread_id": THREAD_IDS["A"]})
            self.record("control_applied", control="finishA", child_total=len(child["messages"]))

    def audit(self, *, unopened=False):
        with self.lock:
            require(not self.errors, "Fixture rejected a request: " + "; ".join(self.errors))
            counts = {key: {"reads": 0, "tail_reads": 0, "older_reads": 0, "revisions": {}}
                      for key in self.children}
            for row in self.requests:
                if row["path"] == "/history/page":
                    key = row["child"]
                    entry = counts[key]
                    entry["reads"] += 1
                    tail = row["before"] is None
                    entry["tail_reads" if tail else "older_reads"] += 1
                    if tail:
                        revision = row["revision"]
                        entry["revisions"][revision] = entry["revisions"].get(revision, 0) + 1
                else:
                    require(row["path"] in ("/bootstrap", "/sessions", "/events", "/config",
                                            "/config/local", "/context", "/resume"),
                            "Unexpected child or mutation route: " + row["path"])
                    require(row.get("session") in (None, self.session), "Child runtime subscription")
                    if row["path"] == "/resume":
                        require(row["body"]["session_dir"] == self.session, "Child runtime resume")
            require(not unopened or all(entry["reads"] == 0 for entry in counts.values()),
                    "Child history was read before the first click")
            return {"passed": True, "children": counts, "child_subscriptions": 0, "child_resumes": 0,
                    "parent_resumes": sum(row["path"] == "/resume" for row in self.requests),
                    "parent_subscriptions": sum(row["path"] == "/events" for row in self.requests)}

    def checkpoint(self, name, previous=None):
        """Compare explicit UI actions; transport cannot infer why a tail was read.

        A manual refresh may legally reread an unchanged revision. Only an
        operator-marked repeated-click interval promises no additional reads.
        """
        require(name in ("check", "check-unopened", "check-repeat", "check-refresh-A", "check-refresh-B"),
                "Unknown checkpoint")
        audit = self.audit(unopened=name == "check-unopened")
        if name == "check-repeat" or name.startswith("check-refresh-"):
            require(previous is not None, "Action checkpoint requires a previous checkpoint")
            expected = copy.deepcopy(previous["audit"])
            if name.startswith("check-refresh-"):
                key = name[-1]
                child = expected["children"][key]
                require(child["tail_reads"] > 0, "Manual refresh requires an initially loaded child")
                # No newer child revision may appear unnoticed between these
                # checkpoints: this action specifically verifies same-revision refresh.
                revision = next(reversed(child["revisions"]))
                child["reads"] += 1
                child["tail_reads"] += 1
                child["revisions"][revision] += 1
            require(audit == expected,
                    "Repeated click changed request counts" if name == "check-repeat" else
                    "Tail refresh must add exactly one same-revision child tail and no other requests")
        return {"name": name, "at_ns": time.monotonic_ns(), "audit": audit}


def record(output, fixture, state):
    with fixture.lock:
        state.update(audit=fixture.audit(), finished_a=fixture.finished_a, bound_a=fixture.bound_a,
                     fixture_errors=list(fixture.errors))
        save(output / "trace.json", fixture.trace)
        save(output / "state.json", state)


def check_fixture(output):
    fixture = SubagentFixture(output, "synthetic-subagent-smoke-token").start()
    try:
        client = build_opener(ProxyHandler({}))
        def get(path, **query):
            req = Request(fixture.origin + path + ("?" + urlencode(query) if query else ""),
                          headers={"Authorization": "Bearer " + fixture.token})
            with client.open(req, timeout=3) as response:
                return json.load(response)
        require(get("/bootstrap")["session_dir"] == fixture.session, "Wrong parent baseline")
        catalog = get("/sessions")
        require(all(row["parent_session_id"] == SESSION_ID for row in catalog[1:]), "Wrong child parents")
        fixture.audit(unopened=True)
        for key in ("A", "B"):
            directory = fixture.children[key]["session"]["session_dir"]
            page = get("/history/page", session_dir=directory)
            require((page["start"], page["total"]) == (100, 150), "Initial page is not bounded tail50")
            rows = page["messages"]
            while page["start"]:
                page = get("/history/page", session_dir=directory, before=page["start"], revision=page["revision"])
                rows = page["messages"] + rows
            require(len(rows) == 150 and all(f"CHILD {key} ONLY" in row["text"] for row in rows),
                    "Child histories crossed or pagination lost messages")
        fixture.finish_a()
        page = get("/history/page", session_dir=fixture.children["A"]["session"]["session_dir"])
        require(page["total"] == 151 and "A FINAL" in page["messages"][-1]["text"], "Missing final append")
        baseline = fixture.checkpoint("check")
        repeated = fixture.checkpoint("check-repeat", baseline)
        get("/history/page", session_dir=fixture.children["A"]["session"]["session_dir"])
        manual = fixture.checkpoint("check-refresh-A", repeated)
        record(output, fixture, {"status": "fixture-smoke-passed", "native_verified": False,
                                 "session": fixture.session, "checks": [baseline, repeated, manual]})
        print("PASS fixture: exact child targets, 50-row pages, final append, same-revision tail; no GUI checked")
    finally:
        fixture.close()


def hold(args):
    fixture = SubagentFixture(args.output, secrets.token_hex(24), deferred_a=args.deferred_a).start()
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
                                      "workspace": fixture.workspace, "token": fixture.token})
    state = {"status": "serving", "session": fixture.session, "origin": fixture.origin,
             "native_verified": False, "checks": [],
             "source_sha256": {name: digest(Path(__file__).with_name(name)) for name in
                               ("subagent_demo.py", "fixture_server.py", "run.py")}}
    def on_signal(_signum, _frame):
        nonlocal interrupted
        interrupted = True
    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        if argv:
            with (args.output / "native.log").open("w") as log:
                process = subprocess.Popen(argv, env={**os.environ, **env}, stdout=log, stderr=log,
                                           start_new_session=True)
            state.update(pid=process.pid, launcher=str(args.native), launcher_sha256=digest(args.native))
        print("SERVING " + json.dumps({"origin": fixture.origin, "launch": str(args.output / "launch.json")}), flush=True)
        while not interrupted and not (args.output / "control/stop").exists():
            if process and process.poll() is not None:
                raise RuntimeError(f"Native exited early ({process.returncode}); see native.log")
            if time.monotonic() - started > args.timeout:
                raise TimeoutError("Subagent demo deadline reached")
            if fixture.connected.is_set() and state["status"] == "serving":
                if process:
                    state.update(launched_executable(process.pid))
                state["status"] = "connected"
                print("READY · parent SSE connected; check the native window separately", flush=True)
            for name, action in (("bindA", fixture.bind_a), ("appendA", fixture.append_a)):
                control = args.output / "control" / name
                if control.exists():
                    control.unlink()
                    action()
                    print("APPLIED " + name, flush=True)
            control = args.output / "control/finishA"
            if control.exists():
                control.unlink()
                fixture.finish_a()
                print("FINISHED A · child append and parent event emitted", flush=True)
            for name in ("check-unopened", "check", "check-repeat", "check-refresh-A", "check-refresh-B"):
                control = args.output / "control" / name
                if control.exists():
                    control.unlink()
                    previous_check = state["checks"][-1] if state["checks"] else None
                    checkpoint = fixture.checkpoint(name, previous_check)
                    state["checks"].append(checkpoint)
                    print("CHECK " + json.dumps(checkpoint), flush=True)
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
            save(args.output / "trace.json", fixture.trace)
            state.update(fixture_errors=fixture.errors, elapsed_s=time.monotonic() - started)
            save(args.output / "state.json", state)
            for sig, handler in previous.items():
                signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, help="Ready Native; omitted for server-only hold")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--check-fixture", action="store_true")
    parser.add_argument("--deferred-a", action="store_true",
                        help="Publish running A only after control/bindA; appendA writes without parent events")
    args = parser.parse_args()
    if not 10 <= args.timeout <= 600:
        parser.error("--timeout must be between 10 and 600 seconds")
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
