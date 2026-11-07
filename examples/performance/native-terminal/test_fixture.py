"""Strict Settings DTO and owned process lifecycle tests; no GUI/Cargo/Core."""
from argparse import Namespace
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
from tempfile import TemporaryDirectory
import time
import unittest
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

from check import check, identity, owned_by
from fixture import TerminalFixture, addon_settings, base, builder
from run import prepare

HERE = Path(__file__).resolve().parent
VERIFY = HERE.parents[2] / "build/verification"


class FixtureTests(unittest.TestCase):
    def setUp(self):
        VERIFY.mkdir(parents=True, exist_ok=True)
        self.temporary = TemporaryDirectory(prefix="native-terminal-test-", dir=VERIFY)
        self.root = Path(self.temporary.name)
        self.fixture = TerminalFixture(self.root, "synthetic-token").start()
        self.client = build_opener(ProxyHandler({}))

    def tearDown(self):
        self.fixture.close()
        self.temporary.cleanup()

    def request(self, endpoint, *, query=None, token="synthetic-token", method="GET"):
        if query is None:
            query = {"session_dir": self.fixture.session}
        request = Request(self.fixture.origin + endpoint + "?" + urlencode(query),
                          headers={"Authorization": "Bearer " + token}, method=method)
        return self.client.open(request, timeout=3)

    def test_settings_builder_complete_pinned_shape(self):
        with self.request("/config/builder") as response:
            value = json.load(response)
        self.assertEqual(value, builder())
        self.assertEqual(set(value), {"addon_settings", "config_path", "target_path", "writable",
            "active_provider", "providers", "model_modules", "permission_mode", "permission_modes",
            "active_modules", "hooks", "hook_modules", "module_config", "tools_enabled", "tools",
            "plugins", "slots", "warnings"})
        self.assertFalse(value["writable"])
        value["addon_settings"]["addons"]["disabled_skills"].append("mutated")
        self.assertEqual(builder()["addon_settings"], addon_settings())

    def test_addons_history_and_startup_reads(self):
        with self.request("/addons") as response:
            value = json.load(response)
        self.assertEqual(value, {"reload_error": None, "writable": False, "settings": addon_settings(),
                                "catalogs": [], "mcp_servers": [], "plugins": []})
        expected = {"/config/history": {"revisions": []}, "/model/quota": None,
                    "/sessions/current": self.fixture.session_summaries()}
        for endpoint, value in expected.items():
            with self.subTest(endpoint=endpoint), self.request(endpoint) as response:
                self.assertEqual(json.load(response), value)
        with self.request("/history/tool-state", query={"session_dir": self.fixture.session,
                                                       "name": "update_plan"}) as response:
            self.assertEqual(json.load(response), {"revision": "terminal-fixture", "tool": None})
        self.assertFalse(self.fixture.errors)

    def test_snapshot_uses_exact_synthetic_session(self):
        with self.request("/events") as stream:
            while line := stream.readline():
                if line.startswith(b"data: "):
                    value = json.loads(line[6:])["event"]["snapshot"]
                    break
            self.fixture.stopping.set()
        self.assertEqual(value["session_id"], base.SESSION_ID)
        self.assertEqual(value["seq"], 0)
        self.assertEqual(value["transcript"], self.fixture.transcript)
        self.assertFalse(value["transcript"][0]["images"])

    def test_auth_rejected_without_poisoning_fixture(self):
        with self.assertRaises(HTTPError) as caught:
            self.request("/addons", token="bad")
        self.assertEqual(caught.exception.code, 401)
        caught.exception.close()
        self.assertFalse(self.fixture.errors)
        self.assertEqual(self.fixture.metrics()["request_count"], 0)

    def test_wrong_session_or_extra_query_is_visible_failure(self):
        with self.assertRaises(HTTPError) as caught:
            self.request("/config/builder", query={"session_dir": "/unowned", "extra": "1"})
        self.assertEqual(caught.exception.code, 400)
        caught.exception.close()
        self.assertTrue(self.fixture.errors)

    def test_settings_are_not_silently_writable(self):
        with self.assertRaises(HTTPError) as caught:
            self.request("/config/builder", method="POST")
        self.assertEqual(caught.exception.code, 400)
        caught.exception.close()
        self.assertTrue(self.fixture.errors)

    def test_prepare_copies_only_supplied_extension_and_private_settings(self):
        package = self.root / "ready-terminal"
        package.mkdir()
        (package / "extension.json").write_text('{"protocol":4,"id":"terminal"}')
        (package / "dummy-binary").write_bytes(b"not-executed")
        args = Namespace(output=self.root / "prepared", terminal=package)
        fixture, env = prepare(args)
        try:
            target = args.output / "extensions/terminal/dummy-binary"
            self.assertEqual(target.read_bytes(), b"not-executed")
            target.write_bytes(b"changed-copy")
            self.assertEqual((package / "dummy-binary").read_bytes(), b"not-executed")
            self.assertEqual(env["HOME"], str(args.output / "home"))
            self.assertEqual(env["SHELL"], "/bin/sh")
            self.assertEqual(env["PROTEUS_EXTENSIONS_DIR"], str(args.output / "extensions"))
            self.assertNotIn(fixture.token, (args.output /
                "xdg-config/dev.proteus.native/preferences.json").read_text())
        finally:
            fixture.server.server_close()


class RunnerTests(unittest.TestCase):
    def setUp(self):
        VERIFY.mkdir(parents=True, exist_ok=True)
        self.temporary = TemporaryDirectory(prefix="native-terminal-runner-", dir=VERIFY)
        self.root = Path(self.temporary.name)
        self.package = self.root / "terminal"
        self.package.mkdir()
        (self.package / "extension.json").write_text('{"protocol":4,"id":"terminal"}')
        self.native = self.root / "synthetic-native.py"
        self.native.write_text(f"#!{sys.executable}\n" + '''
import json, os, signal, subprocess, sys, time
from pathlib import Path
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener
root = Path(sys.argv[1]).parent
origin = sys.argv[sys.argv.index("--origin") + 1]
token = sys.argv[sys.argv.index("--token") + 1]
child = subprocess.Popen([sys.executable, "-c", "import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(300)"], start_new_session=True)
(root / "control/child-pid").write_text(str(child.pid))
client = build_opener(ProxyHandler({}))
query = "?" + urlencode({"session_dir": str(root / "session")})
for endpoint in ("/config/builder", "/addons", "/config/history"):
    with client.open(Request(origin + endpoint + query, headers={"Authorization": "Bearer " + token}), timeout=5) as response:
        json.load(response)
with client.open(Request(origin + "/events" + query, headers={"Authorization": "Bearer " + token}), timeout=5) as stream:
    while not stream.readline().startswith(b"data: "):
        pass
    while not (root / "control/native-exit").exists():
        time.sleep(.01)
    os._exit(int((root / "control/native-exit").read_text()))
''')
        self.native.chmod(0o700)
        self.runner = None

    def tearDown(self):
        if self.runner and self.runner.poll() is None:
            self.runner.terminate()
            self.runner.wait(timeout=12)
        self.temporary.cleanup()

    def launch(self, extra_bootstrap=""):
        output = self.root / "case"
        bootstrap = '''
import run
run.helpers.launched_executable = lambda pid: {"synthetic_native_pid": pid}
''' + extra_bootstrap + '''
try:
    run.main()
except (OSError, RuntimeError, ValueError, TimeoutError) as exc:
    print(f"ERROR: {exc}", flush=True)
    raise SystemExit(1)
'''
        self.runner = subprocess.Popen([sys.executable, "-c", bootstrap,
            "--native", str(self.native), "--terminal", str(self.package), "--output", str(output),
            "--timeout", "15"], cwd=HERE, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if self.runner.poll() is not None:
                self.fail("Early runner exit: " + self.runner.communicate()[0])
            try:
                state = json.loads((output / "state.json").read_text())
            except FileNotFoundError:
                state = {}
            if state.get("status") == "running":
                return output, state["pid"], int((output / "control/child-pid").read_text())
            time.sleep(.025)
        self.fail("Runner failed to reach READY")

    def finish(self, output, native, child):
        log, _ = self.runner.communicate(timeout=12)
        report = json.loads((output / "report.json").read_text())
        self.assertFalse(Path(f"/proc/{native}").exists(), log)
        self.assertFalse(Path(f"/proc/{child}").exists(), log)
        self.assertIn(child, [row["pid"] for row in report["reaped_children"]])
        self.assertFalse(report["fixture_errors"], log)
        return report, log

    def test_control_stop_reaps_native_and_separate_session_child(self):
        output, native, child = self.launch()
        (output / "control/stop").touch()
        report, log = self.finish(output, native, child)
        self.assertEqual(self.runner.returncode, 0, log)
        self.assertEqual(report["status"], "stopped")

    def test_stop_failure_marks_receipt_failed_and_still_reaps_child(self):
        output, native, child = self.launch('''
original_stop = run.helpers.stop_native
def failing_stop(process):
    original_stop(process)
    raise OSError("injected stop failure")
run.helpers.stop_native = failing_stop
''')
        (output / "control/stop").touch()
        report, log = self.finish(output, native, child)
        self.assertEqual(self.runner.returncode, 1, log)
        self.assertEqual(report["status"], "failed")
        self.assertIn("injected stop failure", report["teardown_error"])

    def test_unrequested_exit_fails_and_reaps_orphan(self):
        output, native, child = self.launch()
        (output / "control/native-exit").write_text("7")
        report, log = self.finish(output, native, child)
        self.assertEqual(self.runner.returncode, 1, log)
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["native_exit"], 7)

    def test_shell_proof_checks_owned_identity_and_process_exit(self):
        output, native, child = self.launch()
        (output / "control/shell-pid").write_text(str(child))
        first = check(output, "shell", "alive", remember=True)
        self.assertEqual(first["identity"]["pid"], child)
        self.assertTrue(owned_by(child, native))
        with self.assertRaises(RuntimeError):
            check(output, "shell", "dead")
        # Synthetic owned child only; no Native GUI or user's shell is signalled.
        os.kill(child, signal.SIGKILL)
        deadline = time.monotonic() + 3
        while time.monotonic() < deadline and (row := identity(child)) and row["state"] != "Z":
            time.sleep(.01)
        self.assertTrue(check(output, "shell", "dead")["passed"])
        (output / "control/stop").touch()
        self.finish(output, native, child)

    def test_shell_proof_rejects_unowned_process(self):
        output, native, child = self.launch()
        (output / "control/shell-pid").write_text(str(os.getpid()))
        with self.assertRaises(RuntimeError):
            check(output, "unowned", "alive", remember=True)
        self.assertFalse((output / "shell-unowned.json").exists())
        (output / "control/stop").touch()
        self.finish(output, native, child)

    def test_sigterm_reaps_native_and_owned_orphan(self):
        output, native, child = self.launch()
        os.kill(self.runner.pid, signal.SIGTERM)
        report, log = self.finish(output, native, child)
        self.assertEqual(self.runner.returncode, 0, log)
        self.assertEqual(report["status"], "stopped")


if __name__ == "__main__":
    unittest.main()
