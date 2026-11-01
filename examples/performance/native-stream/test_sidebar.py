"""Sidebar wire/audit checks; native geometry and gestures require the real UI."""
import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

from fixture_server import Fixture, SESSION_ID
from sidebar_demo import SidebarFixture, assert_lazy, check_fixture


class SidebarTests(unittest.TestCase):
    def test_catalog_covers_hierarchy_orphan_and_projects_without_mutable_aliases(self):
        with TemporaryDirectory() as directory:
            fixture = SidebarFixture(Path(directory), "sidebar-test-token")
            try:
                rows = fixture.session_summaries()
                self.assertEqual(len(rows), 6)
                self.assertEqual(rows[0]["session_id"], SESSION_ID)
                self.assertEqual(rows[1]["parent_session_id"], rows[0]["session_id"])
                self.assertEqual(rows[2]["parent_session_id"], rows[1]["session_id"])
                self.assertNotIn(rows[3]["parent_session_id"], {row["session_id"] for row in rows})
                self.assertEqual(len({row["workspace_path"] for row in rows}), 2)
                rows[0]["activity"]["status"] = "running"
                self.assertEqual(fixture.session_summaries()[0]["activity"]["status"], "idle")
                self.assertEqual(len(Fixture.session_summaries(fixture)), 1,
                                 "Base transport must retain its original single-session catalog")
            finally:
                fixture.server.server_close()

    def test_smoke_records_only_main_resume_and_no_native_claim(self):
        with TemporaryDirectory() as directory:
            output = Path(directory)
            check_fixture(output)
            state = json.loads((output / "state.json").read_text())
            self.assertEqual(state["status"], "fixture-smoke-passed")
            self.assertFalse(state["native_verified"])
            self.assertEqual(state["audit"], {"passed": True, "request_count": 3,
                                             "resumes": 1, "subscriptions": 0, "catalog_reads": 1})

    def test_child_history_is_rejected_instead_of_serving_main_transcript(self):
        with TemporaryDirectory() as directory:
            fixture = SidebarFixture(Path(directory), "sidebar-test-token").start()
            try:
                client = build_opener(ProxyHandler({}))
                path = "/events?" + urlencode({"session_dir": fixture.catalog[1]["session_dir"]})
                request = Request(fixture.origin + path,
                                  headers={"Authorization": "Bearer sidebar-test-token"})
                with self.assertRaises(HTTPError) as caught:
                    client.open(request, timeout=3)
                self.assertEqual(caught.exception.code, 400)
                caught.exception.close()
                with self.assertRaisesRegex(RuntimeError, "rejected"):
                    assert_lazy(fixture.requests, fixture.session, fixture.errors)
            finally:
                fixture.close()

    def test_audit_rejects_creation_and_child_resume_or_subscription(self):
        for row in ({"path": "/new-session"},
                    {"path": "/resume", "body": {"session_dir": "child"}},
                    {"path": "/events", "session": "child"}):
            with self.subTest(row=row), self.assertRaises(RuntimeError):
                assert_lazy([row], "main")
        self.assertTrue(assert_lazy([{"path": "/sessions"},
                                     {"path": "/resume", "body": {"session_dir": "main"}},
                                     {"path": "/events", "session": "main"}], "main")["passed"])


if __name__ == "__main__":
    unittest.main()
