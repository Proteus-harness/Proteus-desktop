"""Corpus, real JSONL contributor, snapshot wire and owned lifecycle tests; no GUI/Cargo."""
from argparse import Namespace
import json
from pathlib import Path
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener
import xml.etree.ElementTree as ET

from corpus import LANGUAGE, STAGES, hashes, manifest, markdown, source, stage_from_source, svgs
from fixture import SvgFixture, base, transcript
from renderer import response
from run import prepare, read_receipts, sampler

HERE = Path(__file__).resolve().parent
VERIFY = HERE.parents[2] / "build/verification"


class CorpusTests(unittest.TestCase):
    def test_unique_deterministic_safe_svg_sources(self):
        self.assertEqual(len(manifest()["stages"]), STAGES)
        all_hashes = []
        for stage in range(STAGES):
            self.assertEqual(stage_from_source(source(stage)), stage)
            self.assertEqual(hashes(stage), hashes(stage))
            all_hashes.extend(hashes(stage))
            for svg in svgs(stage):
                root = ET.fromstring(svg)
                self.assertEqual(root.tag, "{http://www.w3.org/2000/svg}svg")
                self.assertEqual(root.attrib["viewBox"], "0 0 1280 280")
                self.assertLess(len(svg), 4096)
                self.assertNotIn("href", svg)
                self.assertNotIn("script", svg)
        self.assertEqual(len(set(all_hashes)), STAGES * 2)
        for value in ("stage=0", "stage=00\n", "stage=16", "stage=-1"):
            with self.assertRaises(ValueError):
                stage_from_source(value)
        for value in (-1, 16, True, 1.5):
            with self.assertRaises(ValueError):
                source(value)

    def test_stable_message_identity_distinct_stage_payload(self):
        rows = [transcript(stage)[0] for stage in range(STAGES)]
        self.assertEqual(len({row["message_id"] for row in rows}), 1)
        self.assertEqual(len({row["text"] for row in rows}), STAGES)
        self.assertTrue(all(row["role"] == "assistant" and not row["streaming"] for row in rows))

    def test_real_renderer_roundtrip_and_receipts(self):
        with TemporaryDirectory() as directory:
            receipts = Path(directory) / "rendered.jsonl"
            messages = [{"type": "hello", "protocol": 4, "server": {"token": "not-in-receipt"}},
                        {"type": "context", "context": "opaque"}]
            for stage in (0, 1, 15):
                messages.append({"type": "render", "surface": "content", "view": "opaque-view",
                                 "session_dir": "/synthetic", "content": {"language": LANGUAGE,
                                 "source": source(stage), "source_revision": stage + 4}})
            result = subprocess.run([sys.executable, str(HERE / "renderer.py"), str(receipts)],
                                    input="".join(json.dumps(row) + "\n" for row in messages),
                                    capture_output=True, text=True, timeout=3, check=True)
            replies = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual(len(replies), 3)
            for stage, reply in zip((0, 1, 15), replies):
                self.assertEqual(reply["source_revision"], stage + 4)
                self.assertEqual(reply["session_dir"], "/synthetic")
                self.assertEqual(reply["view"], "opaque-view")
                self.assertEqual(reply["root"]["type"], "stack")
                nodes = reply["root"]["children"]
                self.assertEqual([node["type"] for node in nodes], ["svg", "svg"])
                self.assertEqual([node["source"] for node in nodes], svgs(stage))
            self.assertNotIn("not-in-receipt", receipts.read_text())
            rows = read_receipts(Path(directory))
            self.assertEqual([row["stage"] for row in rows], [0, 1, 15])
            with receipts.open("a") as file:
                file.write('{"partial":')
            self.assertEqual(read_receipts(Path(directory)), rows)

    def test_outer_markdown_format_routes_exact_fence_to_real_svg(self):
        for stage in range(STAGES):
            reply, receipt = response({"surface": "content", "view": "chat-reply", "content": {
                "language": "text/markdown", "source": markdown(stage), "source_revision": stage}})
            self.assertEqual(receipt["stage"], stage)
            self.assertEqual(receipt["language"], "text/markdown")
            self.assertEqual([node["source"] for node in reply["root"]["children"]], svgs(stage))
        with self.assertRaises(ValueError):
            response({"surface": "content", "view": "invalid", "content": {
                "language": "text/markdown", "source": "unrelated message"}})

    def test_renderer_rejects_wrong_language(self):
        with self.assertRaises(ValueError):
            response({"surface": "content", "content": {"language": "other", "source": source(0)}})


class WireTests(unittest.TestCase):
    def setUp(self):
        VERIFY.mkdir(parents=True, exist_ok=True)
        self.temp = TemporaryDirectory(prefix="native-svg-test-", dir=VERIFY)
        self.root = Path(self.temp.name)
        self.fixture = SvgFixture(self.root, "fixture-token").start()
        self.client = build_opener(ProxyHandler({}))

    def tearDown(self):
        self.fixture.close()
        self.temp.cleanup()

    def request(self, path):
        return self.client.open(Request(self.fixture.origin + path,
                                headers={"Authorization": "Bearer fixture-token"}), timeout=3)

    def event(self, stream):
        while line := stream.readline():
            if line.startswith(b"data: "):
                return json.loads(line[6:])["event"]["snapshot"]
        self.fail("No snapshot before SSE close")

    def test_wire_all_stages_same_identity_monotonic_seq_then_clear(self):
        with self.request("/events?" + urlencode({"session_dir": self.fixture.session})) as stream:
            first = self.event(stream)
            self.assertEqual(first["seq"], 0)
            self.assertEqual(first["transcript"], transcript(0))
            for stage in range(1, STAGES):
                self.fixture.advance(stage)
                snapshot = self.event(stream)
                self.assertEqual(snapshot["seq"], stage)
                self.assertEqual(snapshot["transcript"], transcript(stage))
                for key in ("session_id", "stream_id", "root_thread_id"):
                    self.assertEqual(snapshot[key], first[key])
            self.fixture.clear()
            final = self.event(stream)
            self.assertEqual(final["seq"], STAGES)
            self.assertEqual(final["transcript"], [])
            with self.assertRaises(RuntimeError):
                self.fixture.clear()
            self.fixture.stopping.set()
        self.assertEqual(self.fixture.errors, [])

    def test_reject_out_of_order_duplicate_and_payload_mutation(self):
        with self.assertRaises(RuntimeError):
            self.fixture.advance(1)  # not connected
        self.fixture.connected.set()
        with self.assertRaises(RuntimeError):
            self.fixture.advance(2)
        self.fixture.advance(1)
        for stage in (0, 1, 3, True):
            with self.assertRaises(RuntimeError):
                self.fixture.advance(stage)
        with self.assertRaises(RuntimeError):
            self.fixture.clear()
        mutated = {"type": "session_snapshot", "snapshot": {
            "session_id": base.SESSION_ID, "stream_id": "native-fixture-incarnation",
            "seq": 2, "root_thread_id": base.THREAD_ID, "transcript": transcript(2),
            "history": None, "execution": {"active": None, "last": None}}}
        with self.assertRaises(RuntimeError):
            self.fixture.emit(mutated)
        self.assertEqual(self.fixture.stage, 1)
        self.assertEqual(self.fixture.seq, 1)

    def test_prepare_isolated_single_package_and_corpus_hash_stable(self):
        output = self.root / "prepared"
        fixture, env = prepare(Namespace(output=output))
        try:
            package = output / "extensions/memory-svg"
            config = json.loads((package / "extension.json").read_text())
            self.assertEqual(config["protocol"], 4)
            self.assertEqual(config["surfaces"], {"content": {"languages": [LANGUAGE], "formats": ["text/markdown"]}})
            self.assertEqual(config["required_features"], ["surfaces.content.v1", "widgets.svg.v1"])
            self.assertEqual(config["command"], [str(Path(sys.executable).resolve()), "renderer.py",
                                                str(output / "rendered.jsonl")])
            self.assertEqual(list((output / "extensions").iterdir()), [package])
            self.assertEqual(json.loads((output / "corpus.json").read_text()), manifest())
            for key in ("HOME", "XDG_CONFIG_HOME", "PROTEUS_HOME", "PROTEUS_EXTENSIONS_DIR"):
                self.assertTrue(Path(env[key]).is_relative_to(output))
            for filename in ("renderer.py", "corpus.py"):
                self.assertEqual((package / filename).read_bytes(), (HERE / filename).read_bytes())
        finally:
            fixture.server.server_close()

    def test_unexpected_native_exit_fails_and_records_provenance(self):
        output = self.root / "failed-native"
        result = subprocess.run([sys.executable, str(HERE / "run.py"), "--native", "/bin/false",
                                 "--output", str(output), "--timeout", "10"],
                                capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        state = json.loads((output / "report.json").read_text())
        self.assertEqual(state["status"], "failed")
        self.assertIn("Native exited without a stop request", state["error"])
        self.assertFalse(Path(f'/proc/{state["pid"]}').exists())
        self.assertEqual(len(state["launcher_sha256"]), 64)
        self.assertEqual(len(state["corpus_sha256"]), 64)
        self.assertFalse(state["native_verified"])
        self.assertNotIn(self.fixture.token, (output / "launch.json").read_text())

    def test_final_sampler_failure_still_closes_fixture_and_records_error(self):
        output = self.root / "sampler-failed"
        bootstrap = '''
import run
Tree, Recorder = run.sampler()
class FailingRecorder(Recorder):
    def sample(self):
        if self.phase == "teardown":
            raise OSError("injected final sample failure")
        return super().sample()
run.sampler = lambda: (Tree, FailingRecorder)
run.main()
'''
        result = subprocess.run([sys.executable, "-c", bootstrap, "--native", "/bin/false",
                                 "--output", str(output), "--timeout", "10"],
                                cwd=HERE, capture_output=True, text=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        state = json.loads((output / "report.json").read_text())
        self.assertEqual(state["status"], "failed")
        self.assertIn("injected final sample failure", state["teardown_error"])
        self.assertIn("Native exited without a stop request", state["error"])
        self.assertFalse(Path(f'/proc/{state["pid"]}').exists())
        with self.assertRaises(OSError):
            build_opener(ProxyHandler({})).open(state["origin"] + "/bootstrap", timeout=1)

    def test_sampler_redacts_token(self):
        Tree, _ = sampler()
        process = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(5)",
                                    "--token", "private-token"], start_new_session=True)
        try:
            tree = Tree(process.pid, root_group="gui")
            row = tree._observe(process.pid, tree.root_start)
            self.assertIsNotNone(row)
            self.assertIn("<redacted>", row["argv"])
            self.assertNotIn("private-token", row["argv"])
            self.assertNotIn("private-token", json.dumps(tree.sample()))
        finally:
            process.terminate()
            process.wait(timeout=3)

    def test_owned_orphan_renderer_is_adopted_and_reaped(self):
        code = """
import json
from pathlib import Path
import subprocess
import sys
from run import enable_subreaper, reap_adopted
enable_subreaper()
script = "import subprocess, sys; p = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'], start_new_session=True); print(p.pid, flush=True)"
parent = subprocess.Popen([sys.executable, '-c', script], stdout=subprocess.PIPE, text=True)
child = int(parent.stdout.readline())
parent.stdout.close()
parent.wait(timeout=3)
reaped = reap_adopted()
assert child in {row['pid'] for row in reaped}, reaped
assert not Path(f'/proc/{child}').exists()
print(json.dumps(reaped))
"""
        result = subprocess.run([sys.executable, "-c", code], cwd=HERE,
                                capture_output=True, text=True, timeout=8, check=True)
        self.assertEqual(len(json.loads(result.stdout)), 1)


if __name__ == "__main__":
    unittest.main()
