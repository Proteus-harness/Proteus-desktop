"""Wire/lifecycle checks only; no GUI, Core build, model or real profile."""
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

from fixture import ChatSpacesFixture
from run import prepare


class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.fixture = ChatSpacesFixture(self.root, "synthetic-test-only").start()
        self.client = build_opener(ProxyHandler({}))

    def tearDown(self):
        self.fixture.close()
        self.temporary.cleanup()

    def request(self, path, body=None, *, token="synthetic-test-only"):
        return self.client.open(Request(
            self.fixture.origin + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"}),
            timeout=3)

    def read(self, path, body=None):
        with self.request(path, body) as response:
            return json.load(response)

    def scoped(self, path, state):
        return path + "?" + urlencode({"session_dir": state.session})

    def first_event(self, state):
        with self.request(self.scoped("/events", state)) as response:
            self.assertEqual(response.readline(), b"event: output\n")
            line = response.readline()
            self.assertTrue(line.startswith(b"data: "))
            return json.loads(line[6:])["event"]["snapshot"]

    def test_catalog_and_bootstrap_preserve_two_roots_and_child_foreign_cwd(self):
        catalog = self.read("/sessions")
        self.assertEqual([row["preview"] for row in catalog],
                         ["CHAT SPACE A", "CHAT SPACE B", "CHAT SPACE childA"])
        self.assertIsNone(catalog[0]["parent_session_id"])
        self.assertIsNone(catalog[1]["parent_session_id"])
        self.assertEqual(catalog[2]["parent_session_id"], catalog[0]["session_id"])
        self.assertNotEqual(catalog[2]["workspace_path"], catalog[0]["workspace_path"])
        self.assertEqual(self.read("/bootstrap")["session_dir"], catalog[0]["session_dir"])

    def test_concurrent_sse_snapshots_and_reconnect_never_share_identity(self):
        states = list(self.fixture.sessions.values())
        with ThreadPoolExecutor(max_workers=3) as pool:
            snapshots = list(pool.map(self.first_event, states))
        for state, snapshot in zip(states, snapshots):
            self.assertEqual(snapshot["session_id"], state.session_id)
            self.assertEqual(snapshot["root_thread_id"], state.thread_id)
            self.assertIn("CHAT SPACE " + state.name + " ·", snapshot["transcript"][0]["text"])
            self.assertEqual(snapshot, self.read(self.scoped("/snapshot", state)))
        self.assertEqual(self.first_event(states[0]), snapshots[0])
        self.assertFalse(self.fixture.errors)

    def test_resume_config_and_context_are_per_session(self):
        for state in list(self.fixture.sessions.values()):
            result = self.read("/resume", {"id": state.name, "session_dir": state.session})
            self.assertTrue(result["ok"])
            self.assertEqual(result["output"], state.opened())
            config = self.read(self.scoped("/config/local", state))
            self.assertEqual(config["session_dir"], state.session)
            self.assertEqual(config["cwd"], state.workspace)
            context = self.read(self.scoped("/context", state))
            self.assertEqual(context["session_id"], state.session_id)
            self.assertEqual(context["workspace_path"], state.workspace)

    def test_create_independent_c_then_delete_without_mutating_other_histories(self):
        before = self.fixture.main.snapshot()
        result = self.read("/new-session", {"id": "create-c", "source_session_dir":
                                          self.fixture.main.session,
                                          "workspace": self.fixture.main.workspace})
        created = self.fixture.sessions[result["output"]["session_dir"]]
        self.assertEqual(created.name, "C")
        self.assertIsNone(created.parent_id)
        self.assertIn("CHAT SPACE C", self.first_event(created)["transcript"][0]["text"])
        self.assertEqual(len(self.read("/sessions")), 4)
        deleted = self.read("/delete-session", {"id": "delete-c", "session_dir": created.session})
        self.assertEqual(deleted["output"], {"deleted": True})
        self.assertEqual(len(self.read("/sessions")), 3)
        self.assertEqual(self.fixture.main.snapshot(), before)

    def test_delete_then_create_preserves_surviving_identity_and_subscription(self):
        def create():
            result = self.read("/new-session", {"id":"new", "source_session_dir":self.fixture.main.session})
            return self.fixture.sessions[result["output"]["session_dir"]]
        first, survivor = create(), create()
        stop = self.fixture.subscription_stop[survivor.session]
        snapshot = survivor.snapshot()
        self.read("/delete-session", {"id":"delete", "session_dir":first.session})
        added = create()
        self.assertNotEqual(added.session, survivor.session)
        self.assertNotEqual(added.session_id, survivor.session_id)
        self.assertNotEqual(added.thread_id, survivor.thread_id)
        self.assertEqual(self.fixture.sessions[survivor.session].snapshot(), snapshot)
        self.assertIs(self.fixture.subscription_stop[survivor.session], stop)
        self.assertFalse(stop.is_set())
        with self.assertRaisesRegex(RuntimeError, "already exists"):
            self.fixture.add(survivor.name, 99)

    def test_deleting_initial_session_does_not_reuse_its_survivors_number(self):
        survivor = next(state for state in self.fixture.sessions.values() if state.name == "childA")
        self.read("/delete-session", {"id":"delete", "session_dir":self.fixture.main.session})
        result = self.read("/new-session", {"id":"new", "workspace":survivor.workspace})
        added = self.fixture.sessions[result["output"]["session_dir"]]
        self.assertNotEqual(added.session_id, survivor.session_id)
        self.assertEqual(len(self.fixture.sessions), 3)

    def test_files_listing_and_content_follow_addressed_root_and_child_cwd(self):
        main = self.fixture.main
        child = next(state for state in self.fixture.sessions.values() if state.name == "childA")
        Path(main.workspace, "main.txt").write_text("ROOT FILE MARKER\n")
        Path(child.workspace, "child.txt").write_text("CHILD FILE MARKER\n")
        for state, filename, marker in ((main, "main.txt", "ROOT"), (child, "child.txt", "CHILD")):
            listing = self.read(self.scoped("/workspace/list", state) + "&path=")
            self.assertEqual([entry["name"] for entry in listing["entries"]], [filename])
            content = self.read(self.scoped("/workspace/file", state) + "&path=" + filename)
            self.assertEqual(content["kind"], "text")
            self.assertIn(marker + " FILE MARKER", content["text"])
            self.assertEqual(content["size"], len(content["text"].encode()))
            catalog = self.read(self.scoped("/sessions/current", state))
            self.assertTrue(all(row["workspace_path"] == state.workspace for row in catalog))
            self.assertIn(state.session, [row["session_dir"] for row in catalog])
            self.assertFalse(self.read(self.scoped("/workspace/changes", state))["repository"])
        with self.assertRaises(HTTPError) as caught:
            self.request(self.scoped("/workspace/file", child) + "&path=main.txt")
        caught.exception.close()
        self.assertFalse(self.fixture.errors)

    def test_startup_badges_accept_only_their_explicit_scoped_queries(self):
        for state in self.fixture.sessions.values():
            self.assertIsNone(self.read(self.scoped("/model/quota", state)))
            plan = self.read(self.scoped("/history/tool-state", state) + "&name=update_plan")
            self.assertIsNone(plan["tool"])
            self.assertIn(state.session_id, plan["revision"])
        self.assertFalse(self.fixture.errors)

    def test_unknown_query_stays_rejected_and_its_path_is_recorded(self):
        path = self.scoped("/model/quota", self.fixture.main) + "&unexpected=1"
        with self.assertRaises(HTTPError) as caught:
            self.request(path)
        self.assertEqual(caught.exception.code, 400)
        caught.exception.close()
        self.assertIn("/model/quota", self.fixture.errors[0])
        received = next(row for row in reversed(self.fixture.trace) if row["kind"] == "http_received")
        self.assertEqual(received["query"]["unexpected"], ["1"])

    def test_wrong_token_is_rejected_without_stopping_valid_transport(self):
        with self.assertRaises(HTTPError) as caught:
            self.request("/sessions", token="wrong-token")
        self.assertEqual(caught.exception.code, 401)
        caught.exception.close()
        self.assertEqual(len(self.read("/sessions")), 3)
        self.assertFalse(self.fixture.errors)

    def test_prepare_separates_config_and_has_no_persisted_attach_credentials(self):
        output = self.root / "prepared"
        fixture, launch = prepare(output, Path("/usr/bin/true"), self.root)
        try:
            prefs = json.loads((output / "xdg-config/dev.proteus.native/preferences.json").read_text())
            self.assertIsNone(prefs["tabs_workspace"])
            root = "chat:" + fixture.main.session
            self.assertEqual(prefs["tabs"]["active"], root)
            self.assertEqual(prefs["tabs"]["roots"][root]["root"]["pane"]["tabs"], [root])
            self.assertNotIn(fixture.token, json.dumps(prefs))
            self.assertEqual(launch["env"]["PROTEUS_CONFIG_HOME"], str(output / "config"))
            self.assertEqual(launch["env"]["PROTEUS_EXTENSIONS_DIR"], str(self.root))
        finally:
            fixture.server.server_close()


if __name__ == "__main__":
    unittest.main()
