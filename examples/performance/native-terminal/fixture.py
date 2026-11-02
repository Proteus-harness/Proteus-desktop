"""Strict synthetic session and settings; no Core, models or user history.

Reuses ../native-stream/fixture_server.py under LICENSE-MIT.fixture,
Copyright (c) 2026 Qweasd123tg. DTOs follow pinned proteus-contracts.
"""
import copy
import importlib.util
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

TRANSPORT = Path(__file__).resolve().parent.parent / "native-stream/fixture_server.py"
spec = importlib.util.spec_from_file_location("native_terminal_transport", TRANSPORT)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)


def addon_settings():
    return {"addons": {"disabled_skills": [], "disabled_mcp_servers": [], "plugins": []},
            "mcp_servers": []}


def builder():
    """Complete ConfigBuilderSnapshot; not an arbitrary empty-object fallback."""
    return {"addon_settings": addon_settings(), "config_path": None, "target_path": None,
            "writable": False, "active_provider": None, "providers": [], "model_modules": [],
            "permission_mode": "normal", "permission_modes": ["normal"],
            "active_modules": [], "hooks": [], "hook_modules": [], "module_config": {},
            "tools_enabled": [], "tools": [], "plugins": [], "slots": [], "warnings": []}


class TerminalFixture(base.Fixture):
    def __init__(self, output, token):
        row = base.message("Synthetic terminal lifecycle fixture. Open Terminal from the + tab menu; "
                           "only the supplied ready Terminal extension is installed.", 100)
        row["timestamp_ms"] = 1793059200000
        super().__init__(Path(output) / "workspace", Path(output) / "session", token, [row])

    def metrics(self):
        with self.lock:
            return {"request_count": len(self.requests), "snapshot_seq": self.seq}

    def handle(self, h, method):
        parsed = urlsplit(h.path)
        routes = {"/addons", "/config/builder", "/config/history", "/model/quota",
                  "/history/tool-state", "/sessions/current"}
        if parsed.path not in routes:
            return super().handle(h, method)
        h.connection.settimeout(15)
        if h.headers.get("Authorization") != "Bearer " + self.token:
            h.reply(401, {"error": "Invalid synthetic token"})
            return
        query = parse_qs(parsed.query, strict_parsing=True, keep_blank_values=True)
        expected = {"session_dir": [self.session]}
        if parsed.path == "/history/tool-state":
            expected["name"] = ["update_plan"]
        base.require(method == "GET" and not parsed.fragment and query == expected,
                     "Invalid scoped terminal fixture query: " + parsed.path)
        with self.lock:
            base.require(len(self.requests) < 4096, "Fixture request cap exceeded")
            self.requests.append(self.record("http_request", method=method, path=parsed.path,
                                             session=self.session))
        if parsed.path == "/addons":
            output = {"reload_error": None, "writable": False, "settings": addon_settings(),
                      "catalogs": [], "mcp_servers": [], "plugins": []}
        elif parsed.path == "/config/builder":
            output = builder()
        elif parsed.path == "/config/history":
            output = {"revisions": []}
        elif parsed.path == "/model/quota":
            output = None
        elif parsed.path == "/history/tool-state":
            output = {"revision": "terminal-fixture", "tool": None}
        else:
            output = self.session_summaries()
        h.reply(200, copy.deepcopy(output))
