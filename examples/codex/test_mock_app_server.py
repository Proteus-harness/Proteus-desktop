"""Run: python3 -m unittest discover -s examples/codex -p 'test_*.py'."""
import json
import queue
import subprocess
import threading
import unittest
from pathlib import Path


class Peer:
    def __init__(self):
        self.process = subprocess.Popen([str(Path(__file__).with_name("mock-app-server.py"))], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
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


if __name__ == "__main__":
    unittest.main()
