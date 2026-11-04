"""Readonly child transport oracles; these tests do not prove native UI behavior."""
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener
import uuid

from fixture_server import SESSION_ID
from subagent_demo import SubagentFixture, check_fixture


class SubagentTests(unittest.TestCase):
    def request(self, fixture, path, *, body=None, **query):
        request = Request(fixture.origin + path + ("?" + urlencode(query) if query else ""),
                          data=None if body is None else json.dumps(body).encode(),
                          headers={"Authorization": "Bearer " + fixture.token,
                                   "Content-Type": "application/json"})
        with build_opener(ProxyHandler({})).open(request, timeout=3) as response:
            return json.load(response)

    def test_binding_publishes_running_child_and_live_reply_needs_no_parent_event(self):
        with TemporaryDirectory() as directory:
            fixture = SubagentFixture(Path(directory), "test", deferred_a=True).start()
            try:
                target = fixture.children["A"]["session"]
                self.assertNotIn(target["session_id"],
                                 [row["session_id"] for row in self.request(fixture, "/sessions")])
                fixture.bind_a()
                rows = self.request(fixture, "/sessions")
                row = next(row for row in rows if row["session_id"] == target["session_id"])
                self.assertEqual(row["activity"]["status"], "running")
                before_events = len([row for row in fixture.trace if row["kind"] == "event_enqueued"])
                initial = self.request(fixture, "/history/page", session_dir=target["session_dir"])
                fixture.append_a()
                live = self.request(fixture, "/history/page", session_dir=target["session_dir"])
                self.assertEqual(live["total"], initial["total"] + 1)
                self.assertEqual(live["base"], {"revision": initial["revision"], "common_prefix": 150})
                self.assertIn("A LIVE", live["messages"][-1]["text"])
                self.assertEqual(before_events,
                                 len([row for row in fixture.trace if row["kind"] == "event_enqueued"]))
                self.assertFalse(fixture.finished_a)
                fixture.audit()
            finally:
                fixture.close()

    def test_snapshot_exact_binding_completed_metadata_and_legacy_unbound(self):
        with TemporaryDirectory() as directory:
            fixture = SubagentFixture(Path(directory), "test")
            try:
                live = fixture.transcript[1]["subagent"]
                completed = fixture.transcript[2]["tool"]
                self.assertEqual(live["session"], fixture.children["A"]["session"])
                self.assertEqual(completed["metadata"]["subagent_session"], fixture.children["B"]["session"])
                self.assertEqual(completed["args"]["agent_type"], live["role"])
                self.assertNotEqual(live["child_thread_id"], live["session"]["session_id"])
                self.assertIsNone(fixture.transcript[3]["subagent"]["session"])
                for child in fixture.children.values():
                    uuid.UUID(child["session"]["session_id"])
                    self.assertTrue(Path(child["session"]["session_dir"]).is_absolute())
                catalog = fixture.session_summaries()
                self.assertEqual([row["parent_session_id"] for row in catalog[1:]], [SESSION_ID] * 2)
                catalog[1]["activity"]["status"] = "corrupted"
                self.assertEqual(fixture.session_summaries()[1]["activity"]["status"], "running")
                self.assertEqual(fixture.audit(unopened=True)["children"]["A"]["reads"], 0)
            finally:
                fixture.server.server_close()

    def test_pages_are_lazy_independent_complete_and_non_aliasing(self):
        with TemporaryDirectory() as directory:
            fixture = SubagentFixture(Path(directory), "test").start()
            try:
                self.request(fixture, "/bootstrap")
                self.request(fixture, "/sessions")
                fixture.audit(unopened=True)
                ids = {}
                for key in ("A", "B"):
                    target = fixture.children[key]["session"]["session_dir"]
                    page = self.request(fixture, "/history/page", session_dir=target)
                    self.assertEqual((page["start"], page["total"], len(page["messages"])), (100, 150, 50))
                    rows = page["messages"]
                    while page["start"]:
                        page = self.request(fixture, "/history/page", session_dir=target,
                                            before=page["start"], revision=page["revision"])
                        rows = page["messages"] + rows
                    self.assertEqual(len(rows), 150)
                    self.assertTrue(all(f"CHILD {key} ONLY" in row["text"] for row in rows))
                    ids[key] = {row["message_id"] for row in rows}
                    self.assertEqual(len(ids[key]), 150)
                self.assertTrue(ids["A"].isdisjoint(ids["B"]))
                audit = fixture.audit()
                self.assertEqual(audit["children"]["A"]["reads"], 3)
                self.assertEqual(audit["children"]["B"]["tail_reads"], 1)
                self.assertEqual((audit["child_resumes"], audit["child_subscriptions"]), (0, 0))
                with self.assertRaisesRegex(RuntimeError, "before the first click"):
                    fixture.audit(unopened=True)
            finally:
                fixture.close()

    def test_finish_is_single_append_not_parent_replacement_and_refresh_has_base(self):
        with TemporaryDirectory() as directory:
            fixture = SubagentFixture(Path(directory), "test").start()
            try:
                target = fixture.children["A"]["session"]["session_dir"]
                initial = self.request(fixture, "/history/page", session_dir=target)
                parent_id, parent_path = SESSION_ID, fixture.session
                fixture.finish_a()
                with self.assertRaisesRegex(RuntimeError, "already applied"):
                    fixture.finish_a()
                updated = self.request(fixture, "/history/page", session_dir=target)
                self.assertEqual(updated["base"], {"revision": initial["revision"], "common_prefix": 150})
                self.assertEqual((updated["start"], updated["total"]), (101, 151))
                self.assertEqual(updated["messages"][-1]["text"], "CHILD A FINAL · final saved assistant answer")
                self.assertEqual(len(fixture.children["B"]["messages"]), 150)
                self.assertEqual((SESSION_ID, fixture.session), (parent_id, parent_path))
                envelope = [row["payload"]["envelope"] for row in fixture.trace
                            if row["kind"] == "event_enqueued" and row["event_type"] == "runtime"]
                self.assertEqual(len(envelope), 1)
                self.assertEqual(envelope[0]["session_id"], parent_id)
                self.assertEqual(envelope[0]["event"]["SubagentFinished"]["child_thread_id"],
                                 fixture.transcript[1]["subagent"]["child_thread_id"])
                self.assertEqual(fixture.audit()["children"]["A"]["tail_reads"], 2)
            finally:
                fixture.close()

    def test_repeated_click_checkpoint_detects_load_but_manual_refresh_allows_it(self):
        with TemporaryDirectory() as directory:
            fixture = SubagentFixture(Path(directory), "test").start()
            try:
                target = fixture.children["A"]["session"]["session_dir"]
                unopened = fixture.checkpoint("check-unopened")
                fixture.checkpoint("check-repeat", unopened)
                self.request(fixture, "/history/page", session_dir=target)
                self.request(fixture, "/history/page",
                             session_dir=fixture.children["B"]["session"]["session_dir"])
                first = fixture.checkpoint("check")
                fixture.checkpoint("check-repeat", first)
                self.request(fixture, "/history/page", session_dir=target)
                self.assertEqual(fixture.audit()["children"]["A"]["tail_reads"], 2,
                                 "General audit must allow explicit same-revision Refresh")
                with self.assertRaisesRegex(RuntimeError, "Repeated click changed"):
                    fixture.checkpoint("check-repeat", first)
                refreshed = fixture.checkpoint("check-refresh-A", first)
                self.assertEqual(refreshed["audit"]["children"]["A"]["tail_reads"], 2)
                fixture.checkpoint("check-repeat", refreshed)
                with self.assertRaisesRegex(RuntimeError, "exactly one"):
                    fixture.checkpoint("check-refresh-B", first)
            finally:
                fixture.close()

    def test_action_checkpoints_require_baseline_and_refresh_only_loaded_target(self):
        with TemporaryDirectory() as directory:
            fixture = SubagentFixture(Path(directory), "test")
            try:
                with self.assertRaisesRegex(RuntimeError, "previous checkpoint"):
                    fixture.checkpoint("check-repeat")
                baseline = fixture.checkpoint("check-unopened")
                with self.assertRaisesRegex(RuntimeError, "initially loaded"):
                    fixture.checkpoint("check-refresh-A", baseline)
                with self.assertRaisesRegex(RuntimeError, "Unknown checkpoint"):
                    fixture.checkpoint("check-invented", baseline)
            finally:
                fixture.server.server_close()

    def test_rejects_unknown_targets_cross_revisions_invalid_queries_and_child_runtime(self):
        cases = ("unknown-child", "relative-child", "cross-revision", "stale-revision", "before-no-revision",
                 "negative-before", "beyond-end", "duplicate-query", "unknown-query", "child-resume", "child-events")
        for case in cases:
            with self.subTest(case=case), TemporaryDirectory() as directory:
                fixture = SubagentFixture(Path(directory), "test").start()
                try:
                    target = fixture.children["A"]["session"]["session_dir"]
                    path, body, query = "/history/page", None, {"session_dir": target}
                    if case == "unknown-child":
                        query["session_dir"] = target + "-not-bound"
                    elif case == "relative-child":
                        query["session_dir"] = "child-A"
                    elif case in ("cross-revision", "stale-revision"):
                        query.update(before=100, revision="child-B-revision-1" if case == "cross-revision" else "old")
                    elif case == "before-no-revision":
                        query["before"] = 100
                    elif case in ("negative-before", "beyond-end"):
                        query.update(before=-1 if case == "negative-before" else 151, revision="child-A-revision-1")
                    elif case == "duplicate-query":
                        path += "?" + urlencode({"session_dir": target}) + "&" + urlencode({"session_dir": target})
                        query = {}
                    elif case == "unknown-query":
                        query["limit"] = 100000
                    elif case == "child-resume":
                        path, body, query = "/resume", {"id": "forbidden", "session_dir": target}, {}
                    elif case == "child-events":
                        path = "/events"
                    with self.assertRaises(HTTPError) as caught:
                        self.request(fixture, path, body=body, **query)
                    self.assertEqual(caught.exception.code, 400)
                    caught.exception.close()
                    self.assertTrue(fixture.errors)
                    self.assertFalse(fixture.connected.is_set())
                    self.assertEqual(len(fixture.children["A"]["messages"]), 150)
                    with self.assertRaisesRegex(RuntimeError, "rejected"):
                        fixture.audit()
                finally:
                    fixture.close()

    def test_smoke_receipt_explicitly_does_not_claim_native_verification(self):
        with TemporaryDirectory() as directory:
            output = Path(directory)
            check_fixture(output)
            state = json.loads((output / "state.json").read_text())
            self.assertFalse(state["native_verified"])
            self.assertEqual(state["status"], "fixture-smoke-passed")
            self.assertEqual(state["audit"]["children"]["A"]["reads"], 5)
            self.assertEqual(state["audit"]["children"]["B"]["reads"], 3)
            self.assertEqual(state["audit"]["parent_subscriptions"], 0)
            self.assertEqual([check["name"] for check in state["checks"]],
                             ["check", "check-repeat", "check-refresh-A"])


if __name__ == "__main__":
    unittest.main()
