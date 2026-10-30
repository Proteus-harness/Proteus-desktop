"""Synthetic public app-server transport; no Core, tools or models.

Adapted from proteus-modules-dot foundation/2026-10-27/
adjacent-text-native-window/fixture_server.py, delivery 638c60b79f7e.
Copyright (c) 2026 Qweasd123tg. MIT license: LICENSE-MIT.fixture.
Current DTO: proteus-contracts app_protocol + domain/events.rs (schema 2).
"""
import copy
import json
import hashlib
import queue
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit
def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def sha256(data):
    return hashlib.sha256(data).hexdigest()

SESSION_ID = "00000000-0000-0000-0000-000000000001"
THREAD_ID = "00000000-0000-0000-0000-000000000010"
TURN_ID = "00000000-0000-0000-0000-000000000020"
MESSAGE_ID = "00000000-0000-0000-0000-000000000100"
OPTIONS = {"intent": None, "permission_mode": None}

def message(text, number, role="assistant", streaming=False):
    return {"message_id": str(uuid.UUID(int=number)), "phase": None, "role": role,
            "text": text, "images": [], "tool": None, "subagent": None,
            "streaming": streaming}

def run_state(run_id, status):
    return {"run_id": run_id, "options": dict(OPTIONS), "status": status, "error": None}

class Fixture:
    def __init__(self, workspace, session, token, transcript):
        self.workspace, self.session, self.token = str(workspace), str(session), token
        self.transcript = copy.deepcopy(transcript)
        self.trace, self.errors, self.requests = [], [], []
        self.lock = threading.RLock()
        self.handlers = 0
        self.quiet = threading.Condition(self.lock)
        self.seq = 0
        self.pending = queue.Queue(maxsize=4096)
        self.connected = threading.Event()
        self.stopping = threading.Event()
        self.run_id = None
        self.active = None
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"
            def log_message(self, *args):
                pass
            def do_GET(self):
                self.handle_request("GET")
            def do_POST(self):
                self.handle_request("POST")
            def handle_request(self, method):
                with fixture.quiet:
                    fixture.handlers += 1
                try:
                    fixture.handle(self, method)
                except (BrokenPipeError, ConnectionResetError):
                    if not fixture.stopping.is_set():
                        fixture.fail("Unexpected fixture connection closure")
                except Exception as exc:
                    fixture.fail(type(exc).__name__ + ": " + str(exc))
                    try:
                        self.reply(400, {"error": "Synthetic fixture rejected request"})
                    except (BrokenPipeError, ConnectionResetError):
                        pass
                finally:
                    with fixture.quiet:
                        fixture.handlers -= 1
                        fixture.quiet.notify_all()
            def reply(self, status, value):
                data = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)
                self.wfile.flush()
                self.close_connection = True

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.server.timeout = 1
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.origin = "http://127.0.0.1:" + str(self.server.server_address[1])

    def record(self, kind, **values):
        with self.lock:
            require(len(self.trace) < 8192, "Fixture trace cap exceeded")
            row = {"trace_seq": len(self.trace) + 1, "at_ns": time.monotonic_ns(),
                   "kind": kind, **values}
            self.trace.append(row)
            return row

    def fail(self, reason):
        with self.lock:
            if len(self.errors) < 20:
                self.errors.append(reason)
            self.stopping.set()

    def start(self):
        self.thread.start()
        self.emit({"type": "session_snapshot", "snapshot": {
            "session_id": SESSION_ID, "stream_id": "native-fixture-incarnation",
            "seq": 0, "root_thread_id": THREAD_ID, "transcript": self.transcript,
            "history": None, "execution": {"active": None, "last": None}}})
        return self

    def close(self):
        self.stopping.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)
        require(not self.thread.is_alive(), "Fixture server did not stop")
        until = time.monotonic() + 16
        with self.quiet:
            while self.handlers and time.monotonic() < until:
                self.quiet.wait(timeout=0.1)
            require(self.handlers == 0, "Fixture handlers did not quiesce")

    def emit(self, event):
        require(not self.stopping.is_set(), "Fixture is stopped")
        require(not (event["type"] == "runtime" and
                     "AssistantMessageCompleted" in event["envelope"]["event"]),
                "Completion would heal the delta oracle")
        require(event["type"] != "session_snapshot" or not self.connected.is_set(),
                "A second snapshot would heal the delta oracle")
        data = json.dumps({"type": "event", "event": event},
                          ensure_ascii=False, separators=(",", ":")).encode()
        require(len(data) <= 1024 * 1024, "Synthetic SSE payload cap exceeded")
        row = self.record("event_enqueued", event_type=event["type"],
                          envelope_seq=event.get("envelope", {}).get("seq"),
                          payload_sha256=sha256(data), payload=event,
                          heals_delta_oracle=False)
        self.pending.put_nowait((data, row["trace_seq"]))
        return row["at_ns"]

    def runtime(self, variant, body, thread_id=THREAD_ID, turn_id=TURN_ID):
        with self.lock:
            self.seq += 1
            seq = self.seq
        return self.emit({"type": "runtime", "envelope": {
            "schema_version": 2, "event_id": str(uuid.UUID(int=10000 + seq)),
            "session_id": SESSION_ID, "thread_id": thread_id, "turn_id": turn_id,
            "seq": seq, "timestamp_ms": 1793059200000,
            "event": {variant: body}}})

    def delta(self, offset, text, message_id=MESSAGE_ID, thread_id=THREAD_ID, turn_id=TURN_ID):
        return self.runtime("AssistantTextDelta", {
            "offset": offset, "message_id": message_id, "phase": None, "text": text},
            thread_id, turn_id)

    def tool(self, name="barrierprobe"):
        return self.runtime("ToolCallRequested", {"call": {
            "id": name + "-call", "name": name, "args": {},
            "surface": "function", "raw_arguments": None}})

    def execution(self, status, terminal=False):
        require(self.run_id is not None, "No active fixture run")
        state = run_state(self.run_id, status)
        self.active = None if terminal else state
        return self.emit({"type": "execution_updated", "execution": {
            "active": self.active, "last": state if terminal else None}})

    def activity(self):
        return {"status": "running" if self.active else "idle",
                "running_runs": 1 if self.active else 0,
                "running_run_ids": [self.run_id] if self.active else [],
                "pending_approvals": 0, "pending_user_inputs": 0}

    def config(self):
        return {"display_text": "Synthetic native fixture", "module_epoch": 0,
                "model_catalog_error": None, "activity": self.activity(),
                "config_path": None, "config_files": [], "cwd": self.workspace,
                "session_dir": self.session, "profile": "native-fixture",
                "model": None, "model_options": [], "session_settings": [],
                "reasoning": {"enabled": False, "effort": None, "effort_options": [],
                              "summary": False, "budget_tokens": None},
                "permission_mode": "normal", "modules": [], "tools_enabled": [],
                "registered_tools": [], "components": []}

    def context(self):
        return {"session_dir": self.session, "session_id": SESSION_ID,
                "workspace_path": self.workspace, "activity": self.activity(),
                "history": {"messages": len(self.transcript), "user_messages": 0,
                            "assistant_messages": len(self.transcript),
                            "system_messages": 0, "tool_results": 0, "estimated_tokens": 0},
                "latest_usage": None, "latest_context": None, "latest_compaction": None,
                "tools": {"requested": 0, "finished": 0, "failed": 0, "names": []},
                "diagnostics": []}

    def handle(self, h, method):
        h.connection.settimeout(15)
        require(h.headers.get("Authorization") == "Bearer " + self.token, "Invalid fixture token")
        parsed = urlsplit(h.path)
        require(not parsed.fragment, "Unexpected URL fragment")
        query = parse_qs(parsed.query, strict_parsing=True)
        path = parsed.path
        scoped = path in ("/events", "/config", "/config/local", "/context", "/cancel")
        require(query == ({"session_dir": [self.session]} if scoped else {}),
                f"Unexpected session query: {path}; keys={sorted(query)}")
        body = None
        if method == "POST":
            require(h.headers.get("Transfer-Encoding") is None, "Chunked request not allowed")
            length = int(h.headers.get("Content-Length", "-1"))
            require(0 <= length <= 65536, "Request size outside fixture bound")
            body = json.loads(h.rfile.read(length))
            require(isinstance(body, dict), "Command body must be an object")
        row = self.record("http_request", method=method, path=path,
                          session=self.session if scoped else None, body=body)
        with self.lock:
            require(len(self.requests) < 1024, "Fixture request count cap exceeded")
            self.requests.append(row)
        if method == "GET" and path == "/events":
            require(not self.connected.is_set(), "Unexpected reconnect in native fixture")
            h.send_response(200)
            h.send_header("Content-Type", "text/event-stream")
            h.send_header("Cache-Control", "no-cache")
            h.send_header("Connection", "close")
            h.end_headers()
            h.wfile.flush()
            self.connected.set()
            while not self.stopping.is_set():
                try:
                    data, trace_seq = self.pending.get(timeout=0.5)
                    h.wfile.write(b"event: output\ndata: " + data + b"\n\n")
                    h.wfile.flush()
                    self.record("event_written", enqueued_trace_seq=trace_seq)
                except queue.Empty:
                    h.wfile.write(b": fixture heartbeat\n\n")
                    h.wfile.flush()
            h.close_connection = True
            return
        if method == "GET":
            if path == "/bootstrap":
                output = {"session_dir": self.session, "cwd": self.workspace}
            elif path == "/sessions":
                output = [{"session_dir": self.session, "session_id": SESSION_ID,
                           "workspace_path": self.workspace, "message_count": len(self.transcript),
                           "updated_at_ms": 1793059200000, "preview": "Synthetic native fixture",
                           "activity": self.activity()}]
            elif path in ("/config", "/config/local"):
                output = self.config()
            elif path == "/context":
                output = self.context()
            else:
                raise ValueError("Unexpected GET " + path)
            h.reply(200, output)
            return
        require(method == "POST", "Unexpected HTTP method")
        if path == "/resume":
            require(set(body) == {"id", "session_dir"} and body["session_dir"] == self.session,
                    "Unexpected resume command")
            output = {"session_dir": self.session, "activity": self.activity()}
        else:
            raise ValueError("Unexpected POST " + path)
        h.reply(200, {"type": "response", "id": body.get("id"), "ok": True,
                      "output": output, "error": None})
