"""Multi-session adapter for the existing MIT native-stream loopback transport.

The HTTP handler/lifecycle is imported, not copied. Each session owns immutable
identity and its own snapshot; concurrent subscriptions never swap root state.
Copyright (c) 2026 Qweasd123tg. See ../native-stream/LICENSE-MIT.fixture.
"""
import copy
import importlib.util
import json
from pathlib import Path
import threading
import uuid
from urllib.parse import parse_qs, urlsplit

import workspace as workspace_files

_BASE_PATH = Path(__file__).resolve().parent.parent / "native-stream/fixture_server.py"
_SPEC = importlib.util.spec_from_file_location("chat_spaces_base_transport", _BASE_PATH)
base = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(base)


def identifier(number):
    return str(uuid.UUID(int=number))


class Session:
    def __init__(self, output, name, number, *, workspace=None, parent=None):
        self.name = name
        self.session = str(output / "sessions" / name)
        self.workspace = str(workspace or output / "workspace")
        self.session_id = identifier(number)
        self.thread_id = identifier(1000 + number)
        self.parent_id = parent
        self.transcript = [base.message(
            f"CHAT SPACE {name} · собственная история {name}.\n"
            f"Рабочая папка: {self.workspace}\n"
            "Файлы и терминал должны использовать папку основного чата области.",
            10000 + number)]
        Path(self.session).mkdir(parents=True, exist_ok=True)
        Path(self.workspace).mkdir(parents=True, exist_ok=True)

    def activity(self):
        return {"status": "idle", "running_runs": 0, "running_run_ids": [],
                "pending_approvals": 0, "pending_user_inputs": 0}

    def snapshot(self):
        return {"session_id": self.session_id, "stream_id": "chat-spaces-" + self.name,
                "seq": 0, "root_thread_id": self.thread_id,
                "transcript": copy.deepcopy(self.transcript), "history": None,
                "execution": {"active": None, "last": None}}

    def opened(self):
        return {"session_dir": self.session, "workspace_path": self.workspace,
                "activity": self.activity()}

    def summary(self):
        return {**self.opened(), "session_id": self.session_id,
                "parent_session_id": self.parent_id, "message_count": len(self.transcript),
                "updated_at_ms": 1793059200000 + int(uuid.UUID(self.session_id)),
                "preview": f"CHAT SPACE {self.name}"}

    # These use only fixed per-session fields and pure read helpers from base.
    config = base.Fixture.config
    context = base.Fixture.context

    def context_snapshot(self):
        result = self.context()
        result["session_id"] = self.session_id
        return result


class ChatSpacesFixture(base.Fixture):
    def __init__(self, output, token):
        self.output = Path(output)
        super().__init__(self.output / "workspace", self.output / "sessions/A", token, [])
        self.sessions = {}
        self.subscription_stop = {}
        self.main = self.add("A", 1)
        self.add("B", 2)
        self.add("childA", 3, workspace=self.output / "child-workspace",
                 parent=self.main.session_id)

    def add(self, name, number, **kwargs):
        state = Session(self.output, name, number, **kwargs)
        with self.lock:
            self.sessions[state.session] = state
            self.subscription_stop[state.session] = threading.Event()
        return state

    def start(self):
        self.thread.start()
        return self

    def session_summaries(self):
        with self.lock:
            return [state.summary() for state in self.sessions.values()]

    def handle(self, h, method):
        h.connection.settimeout(15)
        if h.headers.get("Authorization") != "Bearer " + self.token:
            h.reply(401, {"error": "Invalid fixture token"})
            return
        parsed = urlsplit(h.path)
        base.require(not parsed.fragment, "Unexpected URL fragment")
        query = parse_qs(parsed.query, strict_parsing=True, keep_blank_values=True)
        path = parsed.path
        self.record("http_received", method=method, path=path, query=query)
        workspace_path = path in ("/workspace/list", "/workspace/file", "/workspace/changes")
        scoped = workspace_path or path in ("/events", "/snapshot", "/config", "/config/local",
                                           "/context", "/sessions/current", "/model/quota",
                                           "/history/tool-state")
        state = None
        if scoped:
            allowed = ({"session_dir", "path"} if workspace_path
                       else {"session_dir", "name"} if path == "/history/tool-state"
                       else {"session_dir"})
            base.require("session_dir" in query and set(query) <= allowed
                         and all(len(values) == 1 for values in query.values()),
                         f"Unexpected query for {path}; keys={sorted(query)}")
            if path == "/history/tool-state":
                base.require(query.get("name") == ["update_plan"],
                             "Expected exact update_plan tool-state query")
            with self.lock:
                state = self.sessions.get(query["session_dir"][0])
            if state is None:
                h.reply(404, {"error": "Unknown synthetic session"})
                return
        else:
            base.require(not query, f"Unexpected query for {path}; keys={sorted(query)}")
        body = None
        if method == "POST":
            base.require(h.headers.get("Transfer-Encoding") is None, "Chunked request")
            length = int(h.headers.get("Content-Length", "-1"))
            base.require(0 <= length <= 65536, "Request size outside fixture bound")
            body = json.loads(h.rfile.read(length))
            base.require(isinstance(body, dict), "Expected command object")
        row = self.record("http_request", method=method, path=path,
                          session=state.session if state else None, body=body)
        with self.lock:
            base.require(len(self.requests) < 4096, "Fixture request cap exceeded")
            self.requests.append(row)
        if method == "GET":
            if path == "/events":
                self.events(h, state)
                return
            if path == "/bootstrap":
                result = {"session_dir": self.main.session, "cwd": self.main.workspace}
            elif path == "/sessions":
                result = self.session_summaries()
            elif path == "/model/quota":
                result = None  # Current provider reports no model limits.
            elif path == "/history/tool-state":
                result = {"revision": "chat-spaces:" + state.session_id, "tool": None}
            elif path == "/sessions/current":
                result = [row for row in self.session_summaries()
                          if row["workspace_path"] == state.workspace]
            elif workspace_path:
                relative = query.get("path", [""])[0]
                try:
                    result = (workspace_files.listing(state, relative) if path == "/workspace/list"
                              else workspace_files.read(state, relative) if path == "/workspace/file"
                              else workspace_files.changes())
                except (OSError, ValueError) as exc:
                    h.reply(404, {"error": str(exc)})
                    return
            elif path in ("/config", "/config/local"):
                result = state.config()
            elif path == "/context":
                result = state.context_snapshot()
            elif path == "/snapshot":
                result = state.snapshot()
            else:
                raise ValueError("Unsupported synthetic GET " + path)
            h.reply(200, result)
            return
        if path == "/resume":
            base.require(set(body) <= {"id", "session_dir"}, "Unexpected resume fields")
            with self.lock:
                state = self.sessions.get(body.get("session_dir"))
            if state is None:
                h.reply(404, {"error": "Unknown synthetic session"})
                return
            result = state.opened()
        elif path == "/new-session":
            base.require(set(body) <= {"id", "source_session_dir", "workspace"},
                         "Unexpected new-session fields")
            with self.lock:
                number = len(self.sessions) + 1
                base.require(number <= 16, "Synthetic session limit reached")
                source = self.sessions.get(body.get("source_session_dir"))
                workspace = body.get("workspace") or (source.workspace if source else self.main.workspace)
                # Never create or write outside this fixture, even when UI sends a folder.
                base.require(Path(workspace).resolve().is_relative_to(self.output.resolve()),
                             "New session workspace must belong to fixture")
                state = self.add("C" if number == 4 else f"NEW{number}", number,
                                 workspace=workspace)
            result = state.opened()
        elif path == "/delete-session":
            base.require(set(body) <= {"id", "session_dir"}, "Unexpected delete fields")
            with self.lock:
                state = self.sessions.pop(body.get("session_dir"), None)
                if state:
                    self.subscription_stop[state.session].set()
            result = {"deleted": state is not None}
        else:
            raise ValueError("Unsupported synthetic POST " + path)
        h.reply(200, {"type": "response", "id": body.get("id"), "ok": True,
                      "output": result, "error": None})

    def events(self, h, state):
        """Each subscriber receives its own baseline, including reconnects.

        Fixtures are static: the only following messages are heartbeat comments.
        Closing or switching a UI area is expected and never marks a failure.
        """
        h.send_response(200)
        h.send_header("Content-Type", "text/event-stream")
        h.send_header("Cache-Control", "no-cache")
        h.send_header("Connection", "close")
        h.end_headers()
        payload = {"type": "event", "event": {"type": "session_snapshot",
                                               "snapshot": state.snapshot()}}
        data = json.dumps(payload, ensure_ascii=False, separators=(",", ":")).encode()
        try:
            h.wfile.write(b"event: output\ndata: " + data + b"\n\n")
            h.wfile.flush()
            self.connected.set()
            self.record("snapshot_written", session=state.session,
                        session_id=state.session_id, payload_sha256=base.sha256(data))
            stop = self.subscription_stop[state.session]
            while not self.stopping.wait(.25) and not stop.is_set():
                h.wfile.write(b": chat-spaces heartbeat\n\n")
                h.wfile.flush()
        except (BrokenPipeError, ConnectionResetError):
            self.record("subscription_closed", session=state.session)
        finally:
            h.close_connection = True
