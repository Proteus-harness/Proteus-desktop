#!/usr/bin/env python3
"""Offline, synthetic app-server protocol smoke test of a downloaded Codex CLI.

Requires Linux bubblewrap. Never sets CODEX_HOME or changes HOME: a synthetic
directory is mounted over the inherited HOME inside a private mount namespace.
The network namespace has no external network; no turn/model request is sent.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import threading
import time


def command(binary: Path, home: Path, inherited_home: str) -> list[str]:
    return [
        "bwrap", "--die-with-parent", "--new-session", "--unshare-net",
        "--unshare-pid", "--ro-bind", "/", "/", "--proc", "/proc",
        "--dev", "/dev", "--tmpfs", "/opt", "--bind", str(home), inherited_home,
        "--ro-bind", str(binary), "/opt/codex-internals-probe",
        "--chdir", inherited_home + "/workspace", "/opt/codex-internals-probe",
    ]


class Client:
    def __init__(self, argv: list[str], env: dict[str, str], stderr: Path):
        self.stderr = stderr.open("wb")
        self.process = subprocess.Popen(
            argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self.stderr, env=env, start_new_session=True,
        )
        self.inbox: queue.Queue = queue.Queue(maxsize=64)
        self.notifications: dict[str, int] = {}
        self.sequence = 0
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self):
        try:
            while line := self.process.stdout.readline(4 * 1024 * 1024 + 1):
                if len(line) > 4 * 1024 * 1024:
                    raise RuntimeError("app-server output line exceeded 4 MiB")
                self.inbox.put(json.loads(line), timeout=2)
        except Exception as error:
            self.inbox.put(error, timeout=2)
        finally:
            try:
                self.inbox.put(EOFError("app-server stdout closed"), timeout=2)
            except queue.Full:
                pass

    def notify(self, method: str, params: dict):
        self.process.stdin.write((json.dumps({"method": method, "params": params}) + "\n").encode())
        self.process.stdin.flush()

    def call(self, method: str, params: dict, timeout: float = 20):
        self.sequence += 1
        request_id = self.sequence
        self.process.stdin.write((json.dumps({"id": request_id, "method": method, "params": params}) + "\n").encode())
        self.process.stdin.flush()
        deadline = time.monotonic() + timeout
        while True:
            if time.monotonic() >= deadline:
                raise TimeoutError(method)
            message = self.inbox.get(timeout=max(0.01, deadline - time.monotonic()))
            if isinstance(message, Exception):
                raise message
            if message.get("id") == request_id:
                if "error" in message:
                    raise RuntimeError(f"{method}: {message['error']}")
                return message["result"]
            if "method" in message:
                name = message["method"]
                self.notifications[name] = self.notifications.get(name, 0) + 1

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(self.process.pid, signal.SIGTERM)
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=3)
        self.reader.join(timeout=1)
        self.stderr.close()


def run(args):
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    home = output / "synthetic-home"
    workspace = home / "workspace"
    skill = workspace / ".agents/skills/synthetic-renderer"
    skill.mkdir(parents=True)
    (skill / "SKILL.md").write_text(
        "---\nname: synthetic-renderer\ndescription: Synthetic renderer protocol smoke fixture.\n---\n"
        "No network, tools, or model calls are required.\n"
    )
    codex_dir = home / ".codex"
    codex_dir.mkdir()
    (codex_dir / "config.toml").write_text(
        'model_provider = "offline-probe"\nmodel = "synthetic-offline-model"\n'
        'thread_unload_delay_secs = 0\n'
        '[model_providers.offline-probe]\nname = "Synthetic offline provider"\n'
        'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\nrequires_openai_auth = false\n'
        '[features]\nremote_plugin = false\nplugin_discovery_prefetch = false\n'
        'shell_snapshot = false\n[analytics]\nenabled = false\n'
    )
    inherited_home = os.environ["HOME"]
    env = {key: os.environ[key] for key in ("HOME", "USER", "LANG", "PATH") if key in os.environ}
    argv = command(binary, home, inherited_home)
    version = subprocess.check_output(argv + ["--version"], env=env, timeout=15, text=True).strip()
    help_text = subprocess.check_output(argv + ["app-server", "--help"], env=env, timeout=15, text=True)
    (output / "app-server-help.txt").write_text(help_text)
    with binary.open("rb") as source:
        binary_hash = hashlib.file_digest(source, "sha256").hexdigest()
    assert version == "codex-cli 0.166.0", version
    assert binary_hash == "b8a11fedf24b2dfb59a70c9e52b030f9a508dcd49cd2dd0d41956f4c785156b8", binary_hash
    report = {
        "binary_sha256": binary_hash,
        "version": version, "source_tag": "rust-v0.166.0",
        "source_commit": "6ff1e1ca3b30fc8b914720681fcf5fe62e95f856",
        "isolation": "bubblewrap synthetic home bind, original HOME unchanged, network namespace offline",
        "model_turns": 0, "ram_benchmark": args.sampler is not None,
    }
    client = Client(argv + ["app-server", "--stdio"], env, output / "stderr.log")
    recorder = None
    sample_lock = threading.Lock()
    stop_sampling = threading.Event()
    sampling_thread = None
    if args.sampler:
        spec = importlib.util.spec_from_file_location("proteus_proc_sampler", args.sampler.resolve(strict=True))
        sampler = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(sampler)
        tree = sampler.Tree(client.process.pid, root_group="other", classifier=lambda row: (
            "backend" if "codex" in Path(row.get("exe", "")).name else "other"
        ))
        recorder = sampler.Recorder(tree, output / "samples.jsonl")

        def sample_loop():
            while not stop_sampling.wait(0.1):
                with sample_lock:
                    recorder.sample()

        sampling_thread = threading.Thread(target=sample_loop, daemon=True)
        sampling_thread.start()

    def phase(name, **details):
        if recorder:
            with sample_lock:
                recorder.mark(name, **details)
                row = recorder.sample()
            return {key: row[key] for key in ("pss_kib", "rss_kib", "swap_pss_kib", "process_count")}

    try:
        phase("startup")
        initialized = client.call("initialize", {
            "clientInfo": {"name": "proteus-offline-probe", "title": "Synthetic internals probe", "version": "1"},
            "capabilities": {"experimentalApi": True},
        })
        assert initialized["codexHome"] == inherited_home + "/.codex", initialized
        client.notify("initialized", {})
        report["initialize"] = {"codex_home_is_synthetic_mount": True, "platform": initialized.get("platformOs")}
        skills = client.call("skills/list", {"cwds": [inherited_home + "/workspace"], "forceReload": True})
        names = [s["name"] for entry in skills["data"] for s in entry["skills"]]
        assert "synthetic-renderer" in names, names
        report["skills"] = {"synthetic_fixture_discovered": True, "count": len(names)}
        plugins = client.call("plugin/list", {"cwds": [inherited_home + "/workspace"], "marketplaceKinds": ["local"]})
        report["plugins"] = {"local_only": True, "response_keys": sorted(plugins)}
        report["loaded_initial"] = client.call("thread/loaded/list", {})
        assert report["loaded_initial"]["data"] == [], report["loaded_initial"]
        report["lifecycle"] = []
        for index in range(args.warmup + args.cycles):
            cycle_phase = "warmup" if index < args.warmup else "measurement"
            cycle = index if index < args.warmup else index - args.warmup
            if index == args.warmup:
                phase("post_warmup_idle")
                time.sleep(args.idle)
                report["post_warmup_idle"] = phase("post_warmup_idle_end")
            phase(cycle_phase + "_start", cycle=cycle)
            started = client.call("thread/start", {
                "cwd": inherited_home + "/workspace", "ephemeral": True,
                "baseInstructions": "Synthetic offline fixture; never start a model turn.",
                "approvalPolicy": "never", "sandbox": "read-only",
            })
            thread_id = started["thread"]["id"]
            loaded = client.call("thread/loaded/list", {})
            assert thread_id in loaded["data"], loaded
            phase(cycle_phase + "_loaded", cycle=cycle)
            status = client.call("thread/unsubscribe", {"threadId": thread_id})
            assert status["status"] == "unsubscribed", status
            deadline = time.monotonic() + 10
            polls = 0
            while True:
                remaining = client.call("thread/loaded/list", {})["data"]
                polls += 1
                if thread_id not in remaining:
                    break
                if time.monotonic() > deadline:
                    raise TimeoutError("unsubscribed thread runtime remained loaded")
                time.sleep(0.05)
            negative = client.call("thread/unsubscribe", {"threadId": thread_id})
            assert negative["status"] == "notLoaded", negative
            memory = phase(cycle_phase + "_unloaded", cycle=cycle)
            report["lifecycle"].append({"phase": cycle_phase, "cycle": cycle, "unsubscribe": status["status"], "loaded_after": len(remaining), "polls": polls, "second_unsubscribe": negative["status"], "memory": memory})
        phase("final_idle")
        time.sleep(args.idle)
        report["final_idle"] = phase("final_idle_end")
        report["notifications"] = client.notifications
        report["passed"] = True
    except Exception as error:
        report["passed"] = False
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        client.close()
        report["clean_eof"] = phase("clean_eof")
        if recorder:
            stop_sampling.set()
            sampling_thread.join(timeout=2)
            with sample_lock:
                report["resource_totals"] = tree.totals()
                recorder.close()
        report["exit_code"] = client.process.returncode
        if client.process.returncode != 0 or (report["clean_eof"] and report["clean_eof"]["process_count"]):
            report["passed"] = False
            report["cleanup_error"] = "Nonzero EOF exit or observed surviving process tree"
        (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    if not report["passed"]:
        raise RuntimeError(report.get("cleanup_error", report.get("error")))
    print(json.dumps({
        "report": str(output / "report.json"), "passed": report["passed"],
        "version": version, "warmup": args.warmup, "cycles": args.cycles,
        "post_warmup_idle": report.get("post_warmup_idle"),
        "final_idle": report["final_idle"], "clean_eof": report["clean_eof"],
    }, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True, help="New directory, created exclusively")
    parser.add_argument("--cycles", type=int, choices=range(1, 21), default=3)
    parser.add_argument("--warmup", type=int, choices=range(0, 6), default=0)
    parser.add_argument("--idle", type=float, default=2.0)
    parser.add_argument("--sampler", type=Path, help="Existing Core examples/performance/common/proc_sampler.py")
    run(parser.parse_args())
