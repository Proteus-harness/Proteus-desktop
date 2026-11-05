#!/usr/bin/env python3
"""Paged synthetic chat: mixed row heights, per-turn dates and remote scrollbar seeks."""
import argparse
from datetime import datetime, timedelta
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import time
from urllib.parse import parse_qs, urlsplit

from fixture_server import Fixture, SESSION_ID, THREAD_ID, message, require
from run import digest, launched_executable, stop_native
from status_demo import DIRECTORIES, delivered, record

REVISION = "synthetic-history-v1"
PAGE = 50


def make_rows(now=None):
    now = now or datetime.now().astimezone()
    rows = []
    for turn in range(120):
        date = now if turn == 119 else now - timedelta(days=1)
        stamp = int(date.replace(hour=14, minute=32, second=0, microsecond=0).timestamp() * 1000)
        # A middle band is much taller than the initial viewport used to
        # estimate the global track. Page crossings must keep their anchor.
        paragraphs = 12 if 40 <= turn < 80 else 1
        parts = [("user", f"ХОД {turn + 1:03d} · синтетический вопрос"),
                 ("assistant", f"ХОД {turn + 1:03d} · промежуточный ответ"),
                 ("system", ""), ("system", ""),
                 ("assistant", f"ХОД {turn + 1:03d} · завершённый ответ.\n\n" +
                  "\n\n".join("Текст для проверки прокрутки: строка должна сохранять положение."
                                  for _ in range(paragraphs)))]
        for part, (role, text) in enumerate(parts):
            row = message(text, 1000 + len(rows), role=role)
            row["timestamp_ms"] = stamp + part * 1000
            if part in (2, 3):
                row.update(message_id=None, tool={
                    "call_id": f"history-{turn}-{part}", "name": "read_file",
                    "args": {"path": f"synthetic/{turn}-{part}.txt"},
                    "effective_args": None, "status": "done",
                    "result": "SYNTHETIC RESULT", "metadata": None})
            rows.append(row)
    return rows


class HistoryFixture(Fixture):
    def __init__(self, workspace, session, token, *, delay=0.2):
        super().__init__(workspace, session, token, make_rows())
        self.delay = delay

    def start(self):
        self.thread.start()
        self.emit({"type": "session_snapshot", "snapshot": {
            "session_id": SESSION_ID, "stream_id": "native-fixture-incarnation",
            "seq": 0, "root_thread_id": THREAD_ID, "transcript": self.transcript[-PAGE:],
            "history": {"revision": REVISION, "before": len(self.transcript) - PAGE,
                        "base": None}, "execution": {"active": None, "last": None}}})
        return self

    def handle(self, handler, method):
        parsed = urlsplit(handler.path)
        if parsed.path != "/history/page":
            return super().handle(handler, method)
        require(method == "GET", "History only supports GET")
        require(handler.headers.get("Authorization") == "Bearer " + self.token, "Invalid token")
        query = parse_qs(parsed.query, strict_parsing=True)
        require(set(query) == {"session_dir", "before", "revision"}, "Unexpected history query")
        require(query["session_dir"] == [self.session] and query["revision"] == [REVISION],
                "Wrong history identity")
        require(len(query["before"]) == 1, "Repeated before")
        before = int(query["before"][0])
        require(0 < before <= len(self.transcript), "History bound exceeded")
        start = max(0, before - PAGE)
        self.record("history_page", start=start, before=before)
        time.sleep(self.delay)
        handler.reply(200, {"revision": REVISION, "start": start, "total": len(self.transcript),
                            "messages": self.transcript[start:before], "base": None})


def run(args):
    fixture = HistoryFixture(args.output / "workspace", args.output / "session", secrets.token_hex(24)).start()
    process = None
    interrupted = False
    state = {"status": "starting", "rows": len(fixture.transcript), "page_rows": PAGE,
             "launcher": str(args.native), "launcher_sha256": digest(args.native),
             "source_sha256": {name: digest(Path(__file__).with_name(name)) for name in
                               ("history_demo.py", "fixture_server.py", "run.py", "status_demo.py")}}

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
        with (args.output / "native.log").open("w") as log:
            process = subprocess.Popen([str(args.native), str(args.output / "workspace"),
                                        "--origin", fixture.origin, "--token", fixture.token],
                                       env=env, stdout=log, stderr=log, start_new_session=True)
            state["pid"] = process.pid
            started = time.monotonic()
            while not interrupted and not (args.output / "control/stop").exists():
                if fixture.errors:
                    raise RuntimeError("; ".join(fixture.errors))
                if process.poll() is not None:
                    raise RuntimeError(f"Native exited ({process.returncode}); see native.log")
                if time.monotonic() - started > args.timeout:
                    raise TimeoutError("History demo deadline reached")
                if fixture.connected.is_set() and state["status"] == "starting":
                    delivered(fixture)
                    state.update(launched_executable(process.pid), status="running")
                    print("READY", flush=True)
                record(args.output, fixture, state)
                time.sleep(.1)
        state["status"] = "stopped"
    except BaseException as exc:
        state.update(status="failed", error=str(exc))
        raise
    finally:
        fixture.stopping.set()
        if process is not None:
            stop_native(process)
        fixture.close()
        state["fixture_errors"] = fixture.errors
        record(args.output, fixture, state)
        for sig, handler in previous.items():
            signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=int, default=600)
    args = parser.parse_args()
    args.native = args.native.resolve(strict=True)
    require(args.native.is_file() and os.access(args.native, os.X_OK), "Ready executable required")
    require(10 <= args.timeout <= 600, "Timeout must be 10..600")
    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=False)
    for directory in DIRECTORIES:
        (args.output / directory).mkdir(mode=0o700)
    run(args)


if __name__ == "__main__":
    main()
