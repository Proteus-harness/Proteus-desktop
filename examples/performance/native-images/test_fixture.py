"""Wire/corpus/lifecycle tests; no native GUI, Cargo, Core or real profile."""
from argparse import Namespace
from concurrent.futures import ThreadPoolExecutor
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
from tempfile import TemporaryDirectory
import time
import unittest
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener
import zlib

from fixture import ImageFixture, MAX_ENCODED, base, corpus, validate
from run import prepare, sampler

VERIFY = Path(__file__).resolve().parents[3] / "build/verification"


def png_scanlines(data):
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    offset, compressed, kinds = 8, bytearray(), []
    size = None
    while offset < len(data):
        length = struct.unpack(">I", data[offset:offset + 4])[0]
        kind = data[offset + 4:offset + 8]
        payload = data[offset + 8:offset + 8 + length]
        crc = struct.unpack(">I", data[offset + 8 + length:offset + 12 + length])[0]
        assert crc == zlib.crc32(kind + payload)
        kinds.append(kind)
        if kind == b"IHDR":
            size = struct.unpack(">IIBBBBB", payload)
        elif kind == b"IDAT":
            compressed.extend(payload)
        offset += length + 12
    assert kinds == [b"IHDR", b"IDAT", b"IEND"] and offset == len(data)
    return size, zlib.decompress(compressed)


class FixtureTests(unittest.TestCase):
    def setUp(self):
        VERIFY.mkdir(parents=True, exist_ok=True)
        self.temporary = TemporaryDirectory(prefix="native-images-test-", dir=VERIFY)
        self.root = Path(self.temporary.name)
        self.manifest = corpus(self.root, 4, 64, 32)
        self.fixture = ImageFixture(self.root, "test-only-bearer", self.manifest).start()
        self.client = build_opener(ProxyHandler({}))

    def tearDown(self):
        self.fixture.close()
        self.temporary.cleanup()

    def request(self, path, *, token="test-only-bearer"):
        return self.client.open(Request(self.fixture.origin + path,
                                headers={"Authorization": "Bearer " + token}), timeout=3)

    def scoped(self, endpoint, **query):
        return endpoint + "?" + urlencode({"session_dir": self.fixture.session, **query})

    def read_event(self, stream):
        while line := stream.readline():
            if line.startswith(b"data: "):
                return json.loads(line[6:])["event"]["snapshot"]
        self.fail("SSE closed without snapshot")

    def test_png_crc_dimensions_rgba_and_distinct_content(self):
        self.assertEqual(len({row["sha256"] for row in self.manifest}), 4)
        for row in self.manifest:
            data = Path(row["path"]).read_bytes()
            size, scanlines = png_scanlines(data)
            self.assertEqual(size, (64, 32, 8, 6, 0, 0, 0))
            self.assertEqual(len(scanlines), 32 * (1 + 64 * 4))
            self.assertTrue(all(scanlines[y * 257] == 0 for y in range(32)))
            self.assertEqual(row["sha256"], base.sha256(data))
            self.assertLessEqual(len(data), MAX_ENCODED)

    def test_limits_reject_before_generation(self):
        for values in ((1, 32, 32), (3, 32, 32), (34, 32, 32),
                       (2, 2049, 32), (2, 32, 31), (32, 2048, 2048)):
            with self.assertRaises(ValueError):
                validate(*values)
        validate(16, 2048, 2048)  # exact 256 MiB decoded corpus budget

    def test_binary_response_exact_bytes_and_concurrent_counters(self):
        def fetch(row):
            with self.request(self.scoped("/image", path=row["id"])) as response:
                self.assertEqual(response.headers["Content-Type"], "image/png")
                data = response.read()
                self.assertEqual(len(data), int(response.headers["Content-Length"]))
                self.assertEqual(data, Path(row["path"]).read_bytes())
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(fetch, self.manifest))
        metrics = self.fixture.metrics()
        self.assertEqual(metrics["image_requests"], 4)
        self.assertEqual(metrics["active_image_requests"], 0)
        self.assertGreaterEqual(metrics["max_active_image_requests"], 1)
        self.assertEqual(set(metrics["image_ids"]), {row["id"] for row in self.manifest})
        self.assertFalse(self.fixture.errors)

    def test_bad_auth_unknown_id_do_not_stop_or_count_image_fetch(self):
        for path, token, code in ((self.scoped("/image", path=self.manifest[0]["id"]), "bad", 401),
                                  (self.scoped("/image", path="../../etc/passwd"), "test-only-bearer", 404)):
            with self.assertRaises(HTTPError) as caught:
                self.request(path, token=token)
            self.assertEqual(caught.exception.code, code)
            caught.exception.close()
        self.assertEqual(self.fixture.metrics()["image_requests"], 0)
        self.assertFalse(self.fixture.errors)

    def test_extra_duplicate_or_wrong_session_query_rejected(self):
        # A malformed request must fail visibly rather than silently widen scope.
        with self.assertRaises(HTTPError) as caught:
            self.request(self.scoped("/image", path=self.manifest[0]["id"], unexpected="1"))
        self.assertEqual(caught.exception.code, 400)
        caught.exception.close()
        self.assertEqual(self.fixture.metrics()["image_requests"], 0)
        self.assertTrue(self.fixture.errors)

    def test_initial_immutable_refs_then_explicit_empty_snapshot_new_seq(self):
        self.manifest[0]["name"] = "mutated caller"
        with self.request(self.scoped("/events")) as stream:
            first = self.read_event(stream)
            refs = [image for row in first["transcript"] for image in row["images"]]
            self.assertEqual(len(refs), 4)
            self.assertEqual(refs[0]["name"], "image-00.png")
            self.assertTrue(all(set(ref) == {"id", "name", "mime_type", "path"} for ref in refs))
            self.assertEqual([row["role"] for row in first["transcript"]],
                             ["user", "assistant", "user", "assistant"])
            initial = json.loads(json.dumps(first))
            self.fixture.replace()
            second = self.read_event(stream)
            self.assertEqual(second["seq"], first["seq"] + 1)
            self.assertEqual(second["stream_id"], first["stream_id"])
            self.assertEqual(second["transcript"], [])
            self.assertEqual(first, initial)
            self.assertEqual(self.fixture.initial, initial["transcript"])
            with self.assertRaises(RuntimeError):
                self.fixture.replace()
            self.fixture.stopping.set()

    def test_injected_snapshot_or_runtime_rejected(self):
        with self.assertRaises(ValueError):
            self.fixture.emit({"type": "runtime"})
        with self.assertRaises(RuntimeError):
            self.fixture.emit({"type": "session_snapshot", "snapshot": {
                "session_id": base.SESSION_ID, "root_thread_id": base.THREAD_ID,
                "stream_id": "native-fixture-incarnation", "seq": 500, "transcript": []}})

    def test_oversized_encoded_file_rejected_without_leaking_active_counter(self):
        path = Path(self.manifest[0]["path"])
        with path.open("r+b") as stream:
            stream.truncate(MAX_ENCODED + 1)
        with self.assertRaises(HTTPError) as caught:
            self.request(self.scoped("/image", path=self.manifest[0]["id"]))
        self.assertEqual(caught.exception.code, 400)
        caught.exception.close()
        self.assertEqual(self.fixture.metrics()["active_image_requests"], 0)
        self.assertTrue(self.fixture.errors)

    def test_private_prepare_has_no_credentials_in_preferences(self):
        args = Namespace(output=self.root / "prepared", images=2, width=32, height=32,
                         extensions=None, image_delay_ms=50)
        fixture, env = prepare(args)
        try:
            prefs = (args.output / "xdg-config/dev.proteus.native/preferences.json").read_text()
            self.assertNotIn(fixture.token, prefs)
            self.assertEqual(env["HOME"], str(args.output / "home"))
            self.assertEqual(env["PROTEUS_EXTENSIONS_DIR"], str(args.output / "empty-extensions"))
            self.assertEqual(len(fixture.transcript), 2)
        finally:
            fixture.server.server_close()

    def test_sampler_import_does_not_start_native_or_global_threads(self):
        Tree, Recorder = sampler()
        self.assertEqual(Tree.__name__, "RedactedTree")
        self.assertEqual(Recorder.__name__, "Recorder")


class RunnerLifecycleTests(unittest.TestCase):
    """Real runner/HTTP/process teardown, replacing only Native's GUI/ELF oracle."""

    def setUp(self):
        VERIFY.mkdir(parents=True, exist_ok=True)
        self.temporary = TemporaryDirectory(prefix="native-images-lifecycle-", dir=VERIFY)
        self.root = Path(self.temporary.name)
        self.native = self.root / "synthetic-native.py"
        self.native.write_text(f"#!{sys.executable}\n" + '''
import os
from pathlib import Path
import sys
import time
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

workspace = Path(sys.argv[1])
origin = sys.argv[sys.argv.index("--origin") + 1]
token = sys.argv[sys.argv.index("--token") + 1]
session = str(workspace.parent / "session")
request = Request(origin + "/events?" + urlencode({"session_dir": session}),
                  headers={"Authorization": "Bearer " + token})
with build_opener(ProxyHandler({})).open(request, timeout=5) as stream:
    while not stream.readline().startswith(b"data: "):
        pass
    control = workspace.parent / "control/native-exit"
    while not control.exists():
        time.sleep(.01)
    os._exit(int(control.read_text()))
''')
        self.native.chmod(0o700)
        self.runner = None

    def tearDown(self):
        if self.runner and self.runner.poll() is None:
            self.runner.terminate()
            try:
                self.runner.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self.runner.kill()
                self.runner.wait(timeout=5)
        self.temporary.cleanup()

    def launch(self):
        output = self.root / "case"
        # All runner lifecycle, fixture, process sampler and process-group
        # teardown execute unchanged in a separate OS process. The synthetic
        # Native is an interpreter, so only its native-ELF provenance check is
        # replaced. No GUI/display/backend/model is needed by this oracle.
        bootstrap = '''
import run
run.helpers.launched_executable = lambda pid: {"synthetic_native_pid": pid}
try:
    run.main()
except (OSError, RuntimeError, ValueError, TimeoutError) as exc:
    print(f"ERROR: {exc}", flush=True)
    raise SystemExit(1)
'''
        self.runner = subprocess.Popen(
            [sys.executable, "-c", bootstrap, "--native", str(self.native),
             "--output", str(output), "--images", "2", "--width", "32", "--height", "32",
             "--image-delay-ms", "0", "--timeout", "15"],
            cwd=Path(__file__).resolve().parent, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.runner.poll() is not None:
                self.fail("Runner exited before READY: " + self.runner.communicate()[0])
            try:
                state = json.loads((output / "state.json").read_text())
            except FileNotFoundError:
                state = {}
            if state.get("status") == "running":
                self.assertEqual(state["stage"], "images")
                return output, state["pid"]
            time.sleep(.025)
        self.fail("Runner did not reach READY")

    def finish(self, output, native_pid):
        log, _ = self.runner.communicate(timeout=10)
        report = json.loads((output / "report.json").read_text())
        self.assertFalse(Path(f"/proc/{native_pid}").exists(), log)
        self.assertEqual(report["active_image_requests"], 0)
        self.assertIn("tree_totals", report)
        return self.runner.returncode, report, log

    def unsolicited_exit(self, code):
        output, pid = self.launch()
        (output / "control/native-exit").write_text(str(code))
        result, report, log = self.finish(output, pid)
        self.assertEqual(result, 1, log)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["native_exit"], code)
        self.assertIn(f"without a stop request ({code})", report["error"])

    def test_zero_exit_after_ready_is_failure(self):
        self.unsolicited_exit(0)

    def test_nonzero_exit_after_ready_is_failure(self):
        self.unsolicited_exit(7)

    def test_control_stop_after_ready_still_succeeds_and_reaps_native(self):
        output, pid = self.launch()
        (output / "control/stop").touch()
        result, report, log = self.finish(output, pid)
        self.assertEqual(result, 0, log)
        self.assertEqual(report["status"], "stopped")
        self.assertNotIn("error", report)

    def test_sigterm_after_ready_still_succeeds_and_reaps_native(self):
        output, pid = self.launch()
        os.kill(self.runner.pid, signal.SIGTERM)
        result, report, log = self.finish(output, pid)
        self.assertEqual(result, 0, log)
        self.assertEqual(report["status"], "stopped")
        self.assertNotIn("error", report)


if __name__ == "__main__":
    unittest.main()
