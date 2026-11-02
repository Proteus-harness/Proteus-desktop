"""Immutable SVG stages on the imported MIT native-stream transport.

HTTP/SSE implementation: ../native-stream/LICENSE-MIT.fixture,
Copyright 2026 Qweasd123tg. No copied HTTP server.
"""
import copy
import importlib.util
import json
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from corpus import STAGES, markdown

TRANSPORT = Path(__file__).resolve().parent.parent / "native-stream/fixture_server.py"
spec = importlib.util.spec_from_file_location("native_svg_transport", TRANSPORT)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def transcript(stage):
    return [base.message(markdown(stage), 100)]


class SvgFixture(base.Fixture):
    def __init__(self, output, token):
        super().__init__(Path(output) / "workspace", Path(output) / "session", token, transcript(0))
        self.stage = 0
        self.cleared = False
        self.snapshot_emissions = 0

    def emit(self, event):
        base.require(event["type"] == "session_snapshot", "Only stage snapshots allowed")
        snapshot = event["snapshot"]
        expected = [] if self.cleared else transcript(self.stage)
        base.require(snapshot == {"session_id": base.SESSION_ID,
                     "stream_id": "native-fixture-incarnation", "seq": self.seq,
                     "root_thread_id": base.THREAD_ID, "transcript": expected,
                     "history": None, "execution": {"active": None, "last": None}}
                     and self.seq == self.snapshot_emissions, "Invalid immutable stage snapshot/cursor")
        base.require(not self.stopping.is_set(), "Fixture stopped")
        data = json.dumps({"type": "event", "event": event}, separators=(",", ":")).encode()
        base.require(len(data) <= 65536, "Snapshot payload exceeds 64 KiB")
        row = self.record("event_enqueued", event_type="session_snapshot", snapshot_seq=self.seq,
                          stage=self.stage, cleared=self.cleared, payload_sha256=base.sha256(data))
        self.pending.put_nowait((data, row["trace_seq"]))
        self.snapshot_emissions += 1
        return row["at_ns"]

    def advance(self, stage):
        with self.lock:
            base.require(type(stage) is int and self.connected.is_set() and not self.cleared
                         and stage == self.stage + 1 and stage < STAGES,
                         "Stage must advance exactly once in order after SSE connects")
            self.stage = stage
            self._snapshot()

    def clear(self):
        with self.lock:
            base.require(self.connected.is_set() and self.stage == STAGES - 1 and not self.cleared,
                         "Clear allowed once after final stage")
            self.cleared = True
            self._snapshot()

    def _snapshot(self):
        self.seq += 1
        self.transcript = [] if self.cleared else transcript(self.stage)
        self.emit({"type": "session_snapshot", "snapshot": {
            "session_id": base.SESSION_ID, "stream_id": "native-fixture-incarnation",
            "seq": self.seq, "root_thread_id": base.THREAD_ID, "transcript": copy.deepcopy(self.transcript),
            "history": None, "execution": {"active": None, "last": None}}})

    def metrics(self):
        with self.lock:
            return {"svg_stage": self.stage, "cleared": self.cleared, "snapshot_seq": self.seq,
                    "request_count": len(self.requests)}

    def handle(self, h, method):
        parsed = urlsplit(h.path)
        if parsed.path not in ("/model/quota", "/history/tool-state", "/sessions/current"):
            return super().handle(h, method)
        h.connection.settimeout(15)
        base.require(h.headers.get("Authorization") == "Bearer " + self.token,
                     "Invalid fixture token")
        query = parse_qs(parsed.query, strict_parsing=True, keep_blank_values=True)
        expected = {"session_dir": [self.session]}
        if parsed.path == "/history/tool-state":
            expected["name"] = ["update_plan"]
        base.require(method == "GET" and not parsed.fragment and query == expected,
                     "Unexpected scoped fixture query")
        row = self.record("http_request", method=method, path=parsed.path, session=self.session)
        with self.lock:
            base.require(len(self.requests) < 1024, "Request cap exceeded")
            self.requests.append(row)
        h.reply(200, None if parsed.path == "/model/quota" else
                {"revision": "svg-fixture", "tool": None} if parsed.path == "/history/tool-state" else
                self.session_summaries())
