"""Run: python3 -m unittest discover -s examples/codex -p 'test_*.py'."""
import json
import os
import queue
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path


class Peer:
    def __init__(self, environment=None):
        env = os.environ.copy()
        for key in ("PROTEUS_CODEX_FIXTURE_STATE", "PROTEUS_CODEX_FIXTURE_DELAYS", "PROTEUS_CODEX_FIXTURE_RPC_LOG"):
            env.pop(key, None)
        env.update(environment or {})
        self.process = subprocess.Popen([str(Path(__file__).with_name("mock-app-server.py"))], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=env)
        self.queue = queue.Queue()
        self.sequence = 0
        self.skipped = []
        def read():
            for line in self.process.stdout:
                self.queue.put(json.loads(line))
        threading.Thread(target=read, daemon=True).start()

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def wait(self, predicate):
        while True:
            value = self.queue.get(timeout=6)
            if predicate(value):
                return value
            self.skipped.append(value)

    def rpc(self, method, params):
        self.sequence += 1
        rid = self.sequence
        self.send({"id": rid, "method": method, "params": params})
        response = self.wait(lambda value: value.get("id") == rid and "method" not in value)
        if "error" in response:
            raise AssertionError(response["error"])
        return response["result"]

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=5)
        self.process.stdout.close()


class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.peer = Peer()
        self.peer.rpc("initialize", {})

    def tearDown(self):
        self.peer.close()

    def test_opaque_threads_pagination_and_catalog(self):
        rows = self.peer.rpc("thread/list", {})["data"]
        self.assertEqual(rows[1]["parentThreadId"], rows[0]["id"])
        first = self.peer.rpc("thread/items/list", {"threadId": rows[0]["id"], "limit": 50})
        second = self.peer.rpc("thread/items/list", {"threadId": rows[0]["id"], "cursor": first["nextCursor"], "limit": 50})
        self.assertEqual(len(first["data"]), 50)
        self.assertFalse({row["item"]["id"] for row in first["data"]} & {row["item"]["id"] for row in second["data"]})
        self.assertEqual(self.peer.rpc("model/list", {})["data"][0]["model"], "fixture-model")

    def test_question_response_stream_and_canonical_history(self):
        thread = self.peer.rpc("thread/start", {"cwd": "/synthetic"})["thread"]["id"]
        turn = self.peer.rpc("turn/start", {"threadId": thread, "input": [{"type": "text", "text": "/question"}]})["turn"]["id"]
        request = self.peer.wait(lambda value: value.get("method") == "item/tool/requestUserInput")
        self.peer.send({"id": request["id"], "result": {"answers": {"choice:opaque": {"answers": ["Первый"]}}}})
        completed = self.peer.wait(lambda value: value.get("method") == "turn/completed")
        self.assertEqual(completed["params"]["turn"]["id"], turn)
        history = self.peer.rpc("thread/items/list", {"threadId": thread})["data"]
        self.assertEqual(sum(row["item"]["type"] == "userMessage" for row in history), 1)
        self.assertTrue(any("Первый" in row["item"].get("text", "") for row in history))

    def test_interrupt_targets_current_opaque_turn(self):
        thread = self.peer.rpc("thread/start", {"cwd": "/synthetic"})["thread"]["id"]
        turn = self.peer.rpc("turn/start", {"threadId": thread, "input": [{"type": "text", "text": "/slow"}]})["turn"]["id"]
        self.peer.rpc("turn/interrupt", {"threadId": thread, "turnId": turn})
        completed = next((value for value in self.peer.skipped if value.get("method") == "turn/completed"), None)
        if completed is None:
            completed = self.peer.wait(lambda value: value.get("method") == "turn/completed")
        self.assertEqual(completed["params"]["turn"]["status"], "interrupted")
        metadata = self.peer.rpc("thread/resume", {"threadId": thread})
        self.assertEqual(metadata["thread"]["status"]["type"], "idle")


class DelayedFixtureTests(unittest.TestCase):
    def setUp(self):
        directory = Path(__file__).resolve().parents[2] / "build" / "verification"
        directory.mkdir(parents=True, exist_ok=True)
        self.directory = tempfile.TemporaryDirectory(prefix="codex-rpc-", dir=directory)
        self.addCleanup(self.directory.cleanup)
        self.log = Path(self.directory.name) / "rpc.jsonl"
        self.state = Path(self.directory.name) / "state.json"

    def launch(self, delays):
        peer = Peer({"PROTEUS_CODEX_FIXTURE_DELAYS": json.dumps(delays),
                     "PROTEUS_CODEX_FIXTURE_RPC_LOG": str(self.log),
                     "PROTEUS_CODEX_FIXTURE_STATE": str(self.state)})
        self.addCleanup(peer.close)
        peer.rpc("initialize", {})
        return peer

    def test_persistent_resumed_thread_never_reuses_turn_or_item_ids(self):
        turns = []
        for _ in range(2):
            peer = Peer({"PROTEUS_CODEX_FIXTURE_STATE": str(self.state)})
            try:
                peer.rpc("initialize", {})
                peer.rpc("thread/resume", {"threadId": "fixture:thread/main"})
                turn = peer.rpc("turn/start", {"threadId": "fixture:thread/main",
                    "input": [{"type": "text", "text": "restart fixture"}]})["turn"]["id"]
                peer.wait(lambda value: value.get("method") == "turn/completed")
                turns.append(turn)
            finally:
                peer.close()
        self.assertEqual(len(set(turns)), 2)
        history = json.loads(self.state.read_text())["fixture:thread/main"]["history"]
        ids = [row["item"]["id"] for row in history]
        self.assertEqual(len(ids), len(set(ids)))

    def test_fast_resume_passes_delayed_resume_and_log_contains_only_metadata(self):
        peer = self.launch({"thread/resume|fixture:thread/main": 0.35})
        peer.send({"id": "slow-a", "method": "thread/resume", "params": {"threadId": "fixture:thread/main"}})
        peer.send({"id": "fast-b", "method": "thread/resume", "params": {"threadId": "fixture:thread/child"}})
        reply = peer.wait(lambda value: value.get("id") in ("slow-a", "fast-b"))
        self.assertEqual(reply["id"], "fast-b")
        self.assertEqual(peer.wait(lambda value: value.get("id") == "slow-a")["result"]["thread"]["id"], "fixture:thread/main")
        peer.rpc("model/list", {})  # Barrier after the delayed response log write.
        rows = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertTrue(all(set(row) == {"method", "threadId", "requestId"} for row in rows))
        completed = [row["requestId"] for row in rows if row["method"] == "thread/resume:response"]
        self.assertEqual(completed, ["fast-b", "slow-a"])
        self.assertEqual(sum(row["method"] == "model/list" for row in rows), 1)

    def test_delayed_catalog_keeps_snapshot_but_delete_updates_persisted_state(self):
        peer = self.launch({"thread/list": 0.35})
        peer.send({"id": "old-list", "method": "thread/list", "params": {}})
        peer.rpc("thread/delete", {"threadId": "fixture:thread/main"})
        stale = peer.wait(lambda value: value.get("id") == "old-list")["result"]["data"]
        self.assertIn("fixture:thread/main", {row["id"] for row in stale})
        current = peer.rpc("thread/list", {})["data"]
        self.assertNotIn("fixture:thread/main", {row["id"] for row in current})
        self.assertNotIn("fixture:thread/main", json.loads(self.state.read_text()))

    def test_delete_cascades_and_notifies_every_actual_descendant(self):
        rows = {
            "root": {"name": "Root", "cwd": "/synthetic", "history": []},
            "child": {"name": "Child", "cwd": "/synthetic", "history": [], "parentThreadId": "root"},
            "grandchild": {"name": "Grandchild", "cwd": "/synthetic", "history": [], "parentThreadId": "child"},
            "other": {"name": "Other", "cwd": "/synthetic", "history": []},
        }
        self.state.write_text(json.dumps(rows))
        peer = self.launch({})
        peer.rpc("thread/delete", {"threadId": "root"})
        notified = {peer.wait(lambda value: value.get("method") == "thread/deleted")["params"]["threadId"] for _ in range(3)}
        self.assertEqual(notified, {"root", "child", "grandchild"})
        self.assertEqual({row["id"] for row in peer.rpc("thread/list", {})["data"]}, {"other"})
        self.assertEqual(set(json.loads(self.state.read_text())), {"other"})

    def test_rejected_delete_keeps_thread_resumable_after_unsubscribe(self):
        self.state.write_text(json.dumps({"root": {"name": "Root", "cwd": "/synthetic", "history": [], "deleteError": "Synthetic attached reference"}}))
        peer = self.launch({})
        peer.rpc("thread/resume", {"threadId": "root", "excludeTurns": True})
        peer.rpc("thread/unsubscribe", {"threadId": "root"})
        with self.assertRaisesRegex(AssertionError, "Synthetic attached reference"):
            peer.rpc("thread/delete", {"threadId": "root"})
        resumed = peer.rpc("thread/resume", {"threadId": "root", "excludeTurns": True})
        self.assertEqual(resumed["thread"]["id"], "root")
        self.assertIn("root", json.loads(self.state.read_text()))


if __name__ == "__main__":
    unittest.main()
