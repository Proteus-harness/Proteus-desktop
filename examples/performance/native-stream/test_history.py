import json
from datetime import datetime, timedelta
from pathlib import Path
import tempfile
import unittest
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

from history_demo import HistoryFixture, PAGE, REVISION, make_rows


class HistoryTests(unittest.TestCase):
    def test_local_replay_serves_the_short_first_page_without_replacing_rows(self):
        rows = make_rows()[-62:]
        with tempfile.TemporaryDirectory() as directory:
            fixture = HistoryFixture(Path(directory) / "workspace", Path(directory) / "session",
                                     "test-token", delay=0, transcript=rows).start()
            try:
                client = build_opener(ProxyHandler({}))
                def get(path, query):
                    return client.open(Request(fixture.origin + path + "?" + urlencode(query),
                        headers={"Authorization": "Bearer test-token"}), timeout=3)
                with get("/events", {"session_dir": fixture.session}) as stream:
                    line = stream.readline()
                    while not line.startswith(b"data: "):
                        line = stream.readline()
                    snapshot = json.loads(line[6:])["event"]["snapshot"]
                    self.assertEqual(snapshot["transcript"], rows[12:])
                    self.assertEqual(snapshot["history"]["before"], 12)
                    with get("/history/page", {"session_dir": fixture.session,
                                              "revision": REVISION, "before": 12}) as response:
                        page = json.load(response)
                    self.assertEqual(page["start"], 0)
                    self.assertEqual(page["total"], 62)
                    self.assertEqual(page["messages"], rows[:12])
                    fixture.stopping.set()
                self.assertEqual(fixture.errors, [])
            finally:
                fixture.close()

    def test_long_tool_chain_crosses_six_pages(self):
        rows = make_rows(long_tools=True)
        self.assertTrue(all(row["tool"] and row["role"] == "system" for row in rows[200:500]))
        self.assertIsNone(rows[199]["tool"])
        self.assertIsNone(rows[500]["tool"])

    def test_unpaged_snapshot_keeps_the_long_reply_and_has_no_cursor(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = HistoryFixture(Path(directory) / "workspace", Path(directory) / "session",
                                     "test-token", delay=0, unpaged=True).start()
            try:
                request = Request(fixture.origin + "/events?" + urlencode({"session_dir": fixture.session}),
                                  headers={"Authorization": "Bearer test-token"})
                with build_opener(ProxyHandler({})).open(request, timeout=3) as stream:
                    line = stream.readline()
                    while not line.startswith(b"data: "):
                        line = stream.readline()
                    snapshot = json.loads(line[6:])["event"]["snapshot"]
                    self.assertIsNone(snapshot["history"])
                    self.assertEqual(len(snapshot["transcript"]), 26)
                    self.assertGreater(len(snapshot["transcript"][5]["text"]), 20000)
                    fixture.stopping.set()
                self.assertEqual(fixture.errors, [])
            finally:
                fixture.close()

    def test_turn_dates_and_heterogeneous_heights(self):
        now = datetime.now().astimezone()
        rows = make_rows(now)
        self.assertEqual(len(rows), 600)
        self.assertEqual([row["role"] for row in rows[-5:]],
                         ["user", "assistant", "system", "system", "assistant"])
        self.assertEqual(datetime.fromtimestamp(rows[-1]["timestamp_ms"] / 1000).date(), now.date())
        self.assertEqual(datetime.fromtimestamp(rows[-6]["timestamp_ms"] / 1000).date(),
                         (now - timedelta(days=1)).date())
        self.assertEqual(rows[254]["text"].count("Текст для проверки"), 12)
        self.assertEqual(rows[-1]["text"].count("Текст для проверки"), 1)
        self.assertTrue(all(row.get("tool", {}).get("status") == "done"
                            for row in rows if row["tool"]))

    def test_snapshot_and_remote_pages_use_same_source(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = HistoryFixture(Path(directory) / "workspace", Path(directory) / "session",
                                     "test-token", delay=0).start()
            try:
                client = build_opener(ProxyHandler({}))

                def get(path, query):
                    request = Request(fixture.origin + path + "?" + urlencode(query),
                                      headers={"Authorization": "Bearer test-token"})
                    return client.open(request, timeout=3)

                with get("/events", {"session_dir": fixture.session}) as stream:
                    line = stream.readline()
                    while not line.startswith(b"data: "):
                        line = stream.readline()
                    snapshot = json.loads(line[6:])["event"]["snapshot"]
                    self.assertEqual(snapshot["transcript"], fixture.transcript[-PAGE:])
                    self.assertEqual(snapshot["history"]["before"], 550)
                    for before in (325, 126, 50):
                        with get("/history/page", {"session_dir": fixture.session,
                                                  "revision": REVISION, "before": before}) as response:
                            page = json.load(response)
                        self.assertEqual(page["start"], before - PAGE)
                        self.assertEqual(page["messages"], fixture.transcript[before - PAGE:before])
                        self.assertEqual(page["total"], 600)
                    fixture.stopping.set()
                self.assertEqual(fixture.errors, [])
            finally:
                fixture.close()


if __name__ == "__main__":
    unittest.main()
