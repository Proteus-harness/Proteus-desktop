"""Transport/control checks; these do not claim to validate native rendering."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory
from types import SimpleNamespace
import unittest
import time
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener

from fixture_server import Fixture, MESSAGE_ID, THREAD_ID, message
from run import advance, check_copy, digest, launched_executable, save, stop_native
from scenario import BOOT, CHILD_ID, EXPECTED, PRIMARY, STAGES, emit_stage


class TransportTests(unittest.TestCase):
    def setUp(self):
        self.directory = TemporaryDirectory()
        self.output = Path(self.directory.name)
        self.fixture = Fixture(self.output / "workspace", self.output / "session",
                               "synthetic-test-token", [message(BOOT, 1)]).start()
        self.client = build_opener(ProxyHandler({}))

    def tearDown(self):
        self.fixture.close()
        self.directory.cleanup()

    def request(self, path, token="synthetic-test-token"):
        return self.client.open(Request(self.fixture.origin + path,
                                       headers={"Authorization": "Bearer " + token}),
                                timeout=3)

    def test_authenticated_sse_order_and_unhealed_byte_expectation(self):
        from urllib.parse import urlencode
        with self.request("/events?" + urlencode({"session_dir": self.fixture.session})) as stream:
            def read_event():
                while True:
                    line = stream.readline()
                    self.assertTrue(line, "SSE closed unexpectedly")
                    if line.startswith(b"data: "):
                        return json.loads(line[6:])["event"]

            snapshot = read_event()
            self.assertEqual(snapshot["snapshot"]["root_thread_id"], THREAD_ID)
            self.assertEqual(snapshot["snapshot"]["transcript"][0]["text"], BOOT)
            events = []
            counts = {"burst": 67, "barrier": 1, "continuation": 3, "hold": 1}
            for stage in STAGES:
                emit_stage(self.fixture, stage)
                events.extend(read_event() for _ in range(counts[stage]))
            envelopes = [event["envelope"] for event in events if event["type"] == "runtime"]
            self.assertEqual([event["seq"] for event in envelopes],
                             list(range(1, len(envelopes) + 1)))
            primary = [event["event"]["AssistantTextDelta"] for event in envelopes
                       if event["thread_id"] == THREAD_ID
                       and "AssistantTextDelta" in event["event"]
                       and event["event"]["AssistantTextDelta"]["message_id"] == MESSAGE_ID]
            self.assertEqual(len(primary), 65)  # 64 burst, one continuation.
            self.assertEqual(primary[32]["offset"], len("Ж00 Ж01 Ж02 Ж03 Ж04 Ж05 Ж06 Ж07 ".encode()))
            self.assertLess(primary[32]["offset"], primary[31]["offset"])
            text = b""
            for delta in primary[:64]:
                text = text[:delta["offset"]] + delta["text"].encode()
            self.assertEqual(text.decode(), PRIMARY)
            child = [event for event in envelopes if event["thread_id"] == CHILD_ID]
            self.assertEqual(len(child), 2)
            self.assertIn("TurnStarted", child[0]["event"])
            self.assertFalse(any("TurnStarted" in event["event"] for event in envelopes
                                 if event["thread_id"] == THREAD_ID))
            self.assertEqual(child[1]["event"]["AssistantTextDelta"]["message_id"], MESSAGE_ID)
            self.assertFalse(any(event["type"] == "session_snapshot" for event in events))
            self.assertFalse(any("AssistantMessageCompleted" in event["event"] for event in envelopes))
            names = [next(iter(event["event"])) for event in envelopes]
            barrier = names.index("ToolCallRequested")
            self.assertEqual(names[barrier - 1], "AssistantTextDelta")
            self.assertEqual(names[barrier + 1:], ["AssistantTextDelta"] * 3 + ["ToolCallRequested"])
            self.assertEqual(envelopes[-1]["event"]["ToolCallRequested"]["call"]["name"],
                             "freezeprobe")
            self.assertEqual(EXPECTED["hold"], EXPECTED["continuation"])
            # Set stop before closing the SSE client: this disconnect is intentional.
            self.fixture.stopping.set()

    def test_transport_rejects_invalid_auth(self):
        with self.assertRaises(HTTPError) as caught:
            self.request("/bootstrap", token="wrong")
        self.assertEqual(caught.exception.code, 400)
        caught.exception.close()
        self.assertTrue(any("Invalid fixture token" in error for error in self.fixture.errors))

    def test_transport_prohibits_healing(self):
        with self.assertRaisesRegex(RuntimeError, "heal"):
            self.fixture.runtime("AssistantMessageCompleted", {"text": PRIMARY})
        self.fixture.connected.set()
        with self.assertRaisesRegex(RuntimeError, "heal"):
            self.fixture.emit({"type": "session_snapshot", "snapshot": {}})


class ControlTests(unittest.TestCase):
    def test_stage_order_and_exact_copy_receipt(self):
        with TemporaryDirectory() as directory:
            output = Path(directory)
            (output / "control").mkdir()
            save(output / "state.json", {"stage": "ready", "status": "running"})
            with self.assertRaisesRegex(RuntimeError, "Next stage"):
                advance(SimpleNamespace(output=output, stage="continuation"))
            advance(SimpleNamespace(output=output, stage="burst"))
            self.assertTrue((output / "control/burst").exists())
            save(output / "state.json", {"stage": "barrier", "status": "running"})
            clipboard = output / "clipboard.txt"
            clipboard.write_bytes(EXPECTED["barrier"]["primary"].encode())
            args = SimpleNamespace(output=output, stage="barrier", row="primary", file=clipboard)
            check_copy(args)
            self.assertTrue(json.loads((output / "copy-barrier-primary.json").read_text())["passed"])
            clipboard.write_bytes(clipboard.read_bytes() + b"\n")
            with self.assertRaisesRegex(RuntimeError, "Clipboard bytes differ"):
                check_copy(args)
            self.assertFalse(json.loads((output / "copy-barrier-primary.json").read_text())["passed"])
            save(output / "state.json", {"stage": "continuation", "status": "running"})
            with self.assertRaisesRegex(ValueError, "Copy is hidden"):
                check_copy(SimpleNamespace(output=output, stage="continuation", row="secondary",
                                           file=clipboard))
            save(output / "state.json", {"stage": "hold", "status": "running"})
            clipboard.write_bytes(EXPECTED["hold"]["secondary"].encode())
            check_copy(SimpleNamespace(output=output, stage="hold", row="secondary", file=clipboard))
            self.assertTrue(json.loads((output / "copy-hold-secondary.json").read_text())["passed"])


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux /proc executable provenance")
class ProvenanceTests(unittest.TestCase):
    def test_wrapper_exec_records_actual_image_separately(self):
        sleeper = Path(shutil.which("sleep")).resolve()
        with TemporaryDirectory() as directory:
            launcher = Path(directory) / "wrapper"
            # A real wrapper exec, without a GUI or product build.
            launcher.write_text("#!/bin/sh\nexec " + str(sleeper) + " 10\n")
            launcher.chmod(0o700)
            process = subprocess.Popen([str(launcher)], start_new_session=True)
            try:
                until = time.monotonic() + 3
                while os.readlink(f"/proc/{process.pid}/exe") != str(sleeper):
                    self.assertIsNone(process.poll())
                    self.assertLess(time.monotonic(), until)
                    time.sleep(.01)
                receipt = launched_executable(process.pid)
                self.assertEqual(receipt["executable"], str(sleeper))
                self.assertEqual(receipt["executable_sha256"], digest(sleeper))
                self.assertNotEqual(receipt["executable_sha256"], digest(launcher))
            finally:
                stop_native(process)

    def test_interpreter_parent_is_not_misattributed_as_native(self):
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(10)"],
                                   start_new_session=True)
        try:
            with self.assertRaisesRegex(RuntimeError, "wrapper must exec Native"):
                launched_executable(process.pid)
        finally:
            stop_native(process)


if __name__ == "__main__":
    unittest.main()
