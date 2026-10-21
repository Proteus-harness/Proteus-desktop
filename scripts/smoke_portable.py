#!/usr/bin/env python3
"""Relocate the actual release archive and exercise only synthetic local sessions."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import subprocess
import tarfile
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parent.parent


def request(origin, token, path, body=None, authenticated=True):
    headers = {"Content-Type": "application/json"}
    if authenticated:
        headers["Authorization"] = "Bearer " + token
    req = urllib.request.Request(origin + path, headers=headers,
        data=None if body is None else json.dumps(body).encode())
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(req, timeout=30) as response:
        content = response.read()
        return json.loads(content) if content else None


def probe(archive, extensions):
    with tempfile.TemporaryDirectory(prefix="proteus-release-smoke-") as temporary:
        work = Path(temporary)
        relocated = work / "relocated"
        relocated.mkdir()
        with tarfile.open(archive) as bundle:
            bundle.extractall(relocated, filter="data")
        portable = relocated / "Proteus"
        source_manifest = json.loads((portable / "share/release-sources.json").read_text())
        env = os.environ.copy()
        for key in list(env):
            if key.startswith("PROTEUS_"):
                env.pop(key)
        env.update({
            "PATH": str(portable / "bin") + os.pathsep + env.get("PATH", ""),
            "HOME": str(work / "home"), "PROTEUS_HOME": str(work / "runtime"),
            "PROTEUS_CONFIG_HOME": str(work / "config"),
            "XDG_CONFIG_HOME": str(work / "xdg-config"),
            "XDG_DATA_HOME": str(work / "xdg-data"),
            "XDG_STATE_HOME": str(work / "xdg-state"),
            "NO_PROXY": "localhost,127.0.0.1,::1", "no_proxy": "localhost,127.0.0.1,::1",
        })
        Path(env["HOME"]).mkdir()
        workspace = work / "workspace"
        workspace.mkdir()
        (workspace / ".git").mkdir()
        (workspace / "AGENTS.md").write_text("SYNTHETIC_RELEASE_INSTRUCTIONS\n")

        def command(args, timeout=90):
            result = subprocess.run(list(map(str, args)), cwd=workspace, env=env,
                                    capture_output=True, text=True, timeout=timeout)
            if result.returncode:
                raise RuntimeError(f"Smoke command failed: {args[0]} ({result.returncode})\n"
                                   f"{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
            return result.stdout

        checks = []
        for executable in ("proteus", "proteus-native"):
            output = command([portable / "bin" / executable, "--version"])
            assert source_manifest["version"] in output, output
        # The real launcher installs profiles under an isolated configuration home;
        # --version exits before opening a window or starting a server.
        assert source_manifest["version"] in command([portable / "proteus-native", "--version"])
        checks.append("relocated_cli_desktop_launcher_versions")
        for name in ("Proteus-core", "Proteus-modules", "Proteus-desktop", "Proteus-desktop-extensions"):
            assert (portable / "share/licenses" / name / "LICENSE").is_file()
        for relative in ("tool/codex-patch", "hook/codex-context-preparation/src/upstream"):
            for notice in ("LICENSE", "NOTICE"):
                assert (portable / "share/licenses/Proteus-modules" / relative / notice).is_file()
        checks.append("root_and_vendored_notices")
        subprocess.run(["python3", str(ROOT / "scripts/test_portable_dcp.py"), str(portable)],
                       cwd=workspace, env=env, check=True, timeout=120)
        checks.append("relocated_dcp_worker_sources_licenses_and_wasm")
        wire = command(["python3", extensions / "tests/wire.py", "--wire", "--packages",
                        portable / "share/extensions"])
        assert [json.loads(line) for line in wire.splitlines() if line.strip()]
        checks.append("packaged_extensions_wire")
        config = work / "config/configs/release-smoke.toml"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text("include = " + json.dumps(str(portable / "share/configs/restricted.config.toml")) + "\n" + '''active_provider = "fake"
[providers.fake]
provider = "fake"
model = "fake-tool-model"
stream = true
[agent_control]
surface = "none"
roles = []
[components.reference-model.exports.model.fake]
[module_config.model.fake]
implementation = "fake"
''')
        with config.open("a") as output:
            for component in ("reference-model", "reference-capabilities", "dcp"):
                output.write(f"\n[components.{component}]\nenv_allowlist = []\n")
                output.write(f"[components.{component}.env]\n")
                for key in ("HOME", "PROTEUS_HOME", "XDG_DATA_HOME"):
                    output.write(key + " = " + json.dumps(env[key]) + "\n")
            for slot, name in (("hook", "hook.dcp"), ("tool", "dcp.tools")):
                output.write(f"\n[module_config.{slot}.{json.dumps(name)}]\nstate_dir = "
                             + json.dumps(str(work / "dcp-state")) + "\n")
        cli = [portable / "bin/proteus", "--config", config, "--cwd", workspace]
        assert "config loaded" in command(cli + ["doctor"])
        assert "готов к запуску" in command(cli + ["inspect", "plan"])
        checks.append("restricted_profile_doctor_and_assembly")
        assert "smoke.txt" in command(cli + ["--new-session", "apply_patch release portable"])
        assert (workspace / "smoke.txt").read_text() == "smoke\n"
        journals = list((work / "config").rglob("journal.jsonl"))
        assert len(journals) == 1, journals
        journal = journals[0]
        records = [json.loads(line) for line in journal.read_text().splitlines()]
        assert "SYNTHETIC_RELEASE_INSTRUCTIONS" in json.dumps(records)
        for hook in ("codex_compact", "hook.dcp"):
            assert any(hook in json.dumps(record) for record in records if "hook_invoked" in json.dumps(record))
        checks.append("restricted_workflow_context_policy_hooks_and_file_turn")
        before = hashlib.sha256(journal.read_bytes()).hexdigest()
        replay = json.loads(command(cli + ["replay", "workflow", journal, "--json"]))
        assert replay["comparison"]["matched"] and replay["source_journal_unchanged"]
        assert hashlib.sha256(journal.read_bytes()).hexdigest() == before
        checks.append("workflow_replay_preserves_journal")
        token = uuid.uuid4().hex
        with (work / "server.log").open("w") as log:
            process = subprocess.Popen(list(map(str, cli + ["server", "http", "--port", "0",
                "--token", token, "--ready-stdout"])), cwd=workspace, env=env,
                stdout=subprocess.PIPE, stderr=log, text=True)
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    if not selector.select(30):
                        raise RuntimeError("Portable HTTP readiness timeout")
                    ready = json.loads(process.stdout.readline())
                assert ready["type"] == "http_ready"
                origin = ready["origin"]
                request(origin, token, "/health", authenticated=False)
                try:
                    request(origin, token, "/sessions", authenticated=False)
                except urllib.error.HTTPError as error:
                    assert error.code in (401, 403)
                else:
                    raise AssertionError("Unauthenticated sessions were accepted")
                query = urllib.parse.urlencode({"session_dir": str(journal.parent)})
                assert "smoke.txt" in json.dumps(request(origin, token, "/history?" + query))
                session = request(origin, token, "/bootstrap")["session_dir"]
                answer = request(origin, token, "/send", {
                    "id": "synthetic-release-http", "text": "release HTTP smoke", "session_dir": session,
                })
                assert "Fake final answer." in json.dumps(answer), answer
                request(origin, token, "/shutdown", {})
                assert process.wait(timeout=15) == 0
                checks.append("authenticated_http_cold_history_and_restricted_turn")
            finally:
                if process.poll() is None:
                    process.terminate()
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        process.kill()
                        process.wait(timeout=5)
        with archive.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        return {"status": "passed", "checks": checks, "source_manifest": source_manifest,
                "archive": archive.name, "archive_sha256": digest,
                "scope": "synthetic headless; native window and live provider not exercised"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--extensions-source", type=Path,
                        default=os.environ.get("PROTEUS_EXTENSIONS_SOURCE", ROOT / "build/sources/extensions"))
    parser.add_argument("--report", type=Path, default=ROOT / "build/verification/release-smoke.json")
    args = parser.parse_args()
    report = probe(args.archive.resolve(), args.extensions_source.resolve())
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: {len(report['checks'])} relocated portable checks; {args.report}")


if __name__ == "__main__":
    main()
