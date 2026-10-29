"""The example keeps commands and session events scoped after a window move."""
import json
from pathlib import Path
import subprocess
import sys
import unittest


class ContextTests(unittest.TestCase):
    def run_messages(self, messages):
        result = subprocess.run(
            [sys.executable, str(Path(__file__).with_name("main.py"))],
            input="".join(json.dumps(message) + "\n" for message in messages),
            capture_output=True, text=True, check=True,
        )
        return [json.loads(line) for line in result.stdout.splitlines()]

    def test_rehosted_command_uses_destination_and_events_stay_scoped(self):
        messages = [
            {"type": "context", "context": "old", "session_dir": "session-a"},
            {"type": "context", "context": "new", "session_dir": "session-b"},
            {"type": "render", "view": "a", "context": "old", "surface": "document", "session_dir": "session-a"},
            {"type": "render", "view": "b", "context": "old", "surface": "document", "session_dir": "session-a"},
            {"type": "rehost", "view": "a", "context": "new"},
            {"type": "event", "context": "old", "session_dir": "session-c", "event": "session_opened"},
            {"type": "action", "view": "a", "action": "open", "sequence": 1},
        ]
        result = self.run_messages(messages)
        self.assertEqual((result[2]["view"], result[2]["session_dir"]), ("b", "session-c"))
        self.assertEqual(result[3], {"type": "open_document", "context": "new", "view": "a", "argument": "клик-0"})
        sessions = {item["view"]: item["session_dir"] for item in result[-2:]}
        self.assertEqual(sessions, {"a": "session-b", "b": "session-c"})


if __name__ == "__main__":
    unittest.main()
