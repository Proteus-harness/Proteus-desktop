#!/usr/bin/env python3
"""Hold ready Native on 16 deterministic SVG revisions; never invokes Cargo."""
import argparse
import ctypes
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import signal
import subprocess
import sys
import time

from corpus import STAGES, manifest
from fixture import SvgFixture, TRANSPORT

HERE = Path(__file__).resolve().parent
STREAM = HERE.parent / "native-stream"
sys.path.insert(0, str(STREAM))
spec = importlib.util.spec_from_file_location("native_svg_launch_helpers", STREAM / "run.py")
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
sys.path.pop(0)
save, digest = helpers.save, helpers.digest
SAMPLER = HERE.parents[3] / "Proteus-core/examples/performance/common/proc_sampler.py"


def sampler():
    spec = importlib.util.spec_from_file_location("native_svg_sampler", SAMPLER)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)

    class RedactedTree(module.Tree):
        def _observe(self, pid, start):
            row = super()._observe(pid, start)
            if row:
                argv = row.get("argv", [])
                for index, arg in enumerate(argv):
                    if index and argv[index - 1] == "--token":
                        argv[index] = "<redacted>"
                    elif arg.startswith("--token="):
                        argv[index] = "--token=<redacted>"
            return row

    return RedactedTree, module.Recorder


def prepare(args):
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for name in ("control", "workspace", "session", "home", "runtime", "config",
                 "xdg-config", "xdg-data", "xdg-cache", "extensions"):
        (args.output / name).mkdir(mode=0o700)
    package = args.output / "extensions/memory-svg"
    package.mkdir(mode=0o700)
    for name in ("renderer.py", "corpus.py"):
        shutil.copyfile(HERE / name, package / name)
    save(package / "extension.json", {
        "protocol": 4, "id": "memory-svg", "title": "Synthetic SVG memory fixture",
        "description": "Deterministic SVG churn; benchmark only", "icon": "chart-column",
        "command": [str(Path(sys.executable).resolve()), "renderer.py", str(args.output / "rendered.jsonl")],
        "surfaces": {"content": {"languages": ["memory-svg"], "formats": ["text/markdown"]}},
        "required_features": ["surfaces.content.v1", "widgets.svg.v1"],
        "author": "Proteus Project"})
    save(args.output / "corpus.json", manifest())
    prefs = args.output / "xdg-config/dev.proteus.native"
    prefs.mkdir(mode=0o700)
    save(prefs / "preferences.json", {"motion": "off", "preferred_agent": "proteus"})
    fixture = SvgFixture(args.output, secrets.token_hex(24))
    env = {key: str(args.output / name) for key, name in (
        ("HOME", "home"), ("PROTEUS_HOME", "runtime"), ("PROTEUS_CONFIG_HOME", "config"),
        ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data"),
        ("XDG_CACHE_HOME", "xdg-cache"), ("PROTEUS_EXTENSIONS_DIR", "extensions"))}
    env.update(SHELL="/bin/sh", PROTEUS_STARTUP_TRACE="1", PYTHONDONTWRITEBYTECODE="1")
    return fixture, env


def read_receipts(output):
    path = output / "rendered.jsonl"
    if not path.exists():
        return []
    if path.stat().st_size > 512 * 4096:
        raise RuntimeError("Renderer receipt file exceeds bound")
    data = path.read_bytes()
    # A producer can be interrupted between write and newline; parse complete lines only.
    lines = data.splitlines() if data.endswith(b"\n") else data.splitlines()[:-1]
    if len(lines) > 512:
        raise RuntimeError("Renderer receipt count exceeds bound")
    rows = [json.loads(line) for line in lines]
    expected = manifest()["stages"]
    for row in rows:
        stage = row["stage"]
        if type(stage) is not int or not 0 <= stage < STAGES or row["svg_sha256"] != expected[stage]["svg_sha256"]:
            raise RuntimeError("Renderer receipt differs from deterministic corpus")
    return rows


def enable_subreaper():
    # Extension processes use their own process group. Adopt/reap them if Native exits first.
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0):  # Linux PR_SET_CHILD_SUBREAPER
        raise OSError(ctypes.get_errno(), "Cannot enable benchmark child subreaper")


def reap_adopted():
    reaped = []
    deadline = time.monotonic() + 4
    while True:
        while True:
            try:
                pid, status = os.waitpid(-1, os.WNOHANG)
            except ChildProcessError:
                return reaped
            if not pid:
                break
            reaped.append({"pid": pid, "wait_status": status})
        children_file = Path(f"/proc/{os.getpid()}/task/{os.getpid()}/children")
        children = [int(pid) for pid in children_file.read_text().split()]
        for pid in children:
            try:
                os.kill(pid, signal.SIGKILL if time.monotonic() > deadline - 2 else signal.SIGTERM)
            except ProcessLookupError:
                pass
        if time.monotonic() > deadline:
            raise RuntimeError("Owned renderer children did not reap")
        time.sleep(.02)


def run(args):
    fixture, env = prepare(args)
    process = tree = recorder = None
    started = time.monotonic()
    interrupted = False
    state = {"status": "starting", "stage": "connecting", "native_verified": False,
             "origin": fixture.origin, "launcher": str(args.native), "launcher_sha256": digest(args.native),
             "source_sha256": {path.name: digest(path) for path in HERE.glob("*.py")},
             "corpus_sha256": digest(args.output / "corpus.json"),
             "extension_manifest_sha256": digest(args.output / "extensions/memory-svg/extension.json"),
             "transport_sha256": digest(TRANSPORT), "launch_helpers_sha256": digest(STREAM / "run.py"),
             "sampler_sha256": digest(SAMPLER), "marked_samples": {}}

    def record(strict=True):
        with fixture.lock:
            state.update(fixture.metrics(), fixture_errors=list(fixture.errors))
            save(args.output / "trace.json", fixture.trace)
        try:
            receipts = read_receipts(args.output)
            state.update(renderer_receipts=len(receipts), rendered_stages=sorted({row["stage"] for row in receipts}))
        except (ValueError, RuntimeError, KeyError, TypeError) as exc:
            if strict:
                raise
            state.update(status="failed", receipt_error=f"{type(exc).__name__}: {exc}")
        save(args.output / "state.json", state)

    def on_signal(_signal, _frame):
        nonlocal interrupted
        interrupted = True

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        enable_subreaper()
        fixture.start()
        argv = [str(args.native), fixture.workspace, "--origin", fixture.origin, "--token", fixture.token]
        save(args.output / "launch.json", {"argv": argv[:-1] + ["<redacted>"], "env": env})
        with (args.output / "native.log").open("w") as log:
            process = subprocess.Popen(argv, env={**os.environ, **env}, cwd=fixture.workspace,
                                       stdout=log, stderr=log, start_new_session=True)
        state["pid"] = process.pid
        Tree, Recorder = sampler()
        tree = Tree(process.pid, root_group="gui")
        recorder = Recorder(tree, args.output / "samples.jsonl")
        recorder.mark("startup")
        record()
        print("STARTED " + json.dumps({"pid": process.pid, "output": str(args.output)}), flush=True)
        marked = set()
        consumed = set()
        while not interrupted and not (args.output / "control/stop").exists():
            if fixture.errors:
                raise RuntimeError("; ".join(fixture.errors))
            if process.poll() is not None:
                state["native_exit"] = process.returncode
                raise RuntimeError(f"Native exited without a stop request ({process.returncode})")
            if time.monotonic() - started > args.timeout:
                raise TimeoutError("SVG fixture deadline reached")
            if fixture.connected.is_set() and state["status"] == "starting":
                state.update(helpers.launched_executable(process.pid), status="running", stage="svg-00")
                recorder.mark("svg-00")
                print("READY · stage 00 snapshot delivered; paint acceptance is separate", flush=True)
            controls = sorted((args.output / "control").glob("next-*"))
            fresh = [control for control in controls if control.name not in consumed]
            if fresh:
                if len(fresh) != 1 or not re.fullmatch(r"next-\d{2}", fresh[0].name):
                    raise RuntimeError("Request one exact next-NN stage at a time")
                stage = int(fresh[0].name[5:])
                delivered = {row["stage"] for row in read_receipts(args.output)}
                if fixture.stage not in delivered:
                    raise RuntimeError("Previous stage has no SVG callback receipt; wait for renderer")
                fixture.advance(stage)
                consumed.add(fresh[0].name)
                state["stage"] = f"svg-{stage:02d}"
                recorder.mark(state["stage"])
            if (args.output / "control/clear").exists() and state["stage"] != "cleared":
                if STAGES - 1 not in {row["stage"] for row in read_receipts(args.output)}:
                    raise RuntimeError("Final stage has no SVG callback receipt; wait for renderer")
                fixture.clear()
                state["stage"] = "cleared"
                recorder.mark("cleared")
            phase_file = args.output / "control/phase"
            if phase_file.exists():
                phase = phase_file.read_text().strip()
                if re.fullmatch(r"[a-zA-Z0-9_-]{1,64}", phase) and phase != recorder.phase:
                    recorder.mark(phase)
            for marker in sorted((args.output / "control").glob("mark-*")):
                label = marker.name[5:]
                if re.fullmatch(r"[a-zA-Z0-9_-]{1,64}", label) and label not in marked:
                    marked.add(label)
                    recorder.mark(label)
                    state["marked_samples"][label] = recorder.sample()
            recorder.sample()
            record()
            time.sleep(.1)
        state["status"] = "stopped"
    except BaseException as exc:
        state.update(status="failed", error=f"{type(exc).__name__}: {exc}")
        raise
    finally:
        try:
            fixture.stopping.set()
            if process:
                helpers.stop_native(process)
                state["reaped_children"] = reap_adopted()
        except BaseException as exc:
            state.update(status="failed", teardown_error=f"{type(exc).__name__}: {exc}")
            raise
        finally:
            if recorder:
                recorder.mark("teardown")
                recorder.sample()
                recorder.close()
                state["tree_totals"] = tree.totals()
            try:
                if fixture.thread.ident is not None:
                    fixture.close()
                else:
                    fixture.server.server_close()
            finally:
                state["elapsed_s"] = time.monotonic() - started
                record(strict=False)
                save(args.output / "report.json", state)
                for sig, handler in previous.items():
                    signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native", type=Path, required=True, help="Ready Native launcher; must exec ELF")
    parser.add_argument("--timeout", type=float, default=1200)
    args = parser.parse_args()
    if not 10 <= args.timeout <= 7200:
        parser.error("--timeout must be between 10 and 7200 seconds")
    args.native = args.native.resolve(strict=True)
    if not args.native.is_file() or not os.access(args.native, os.X_OK):
        parser.error("--native must be an already-built executable")
    args.output = args.output.resolve()
    if not args.output.is_relative_to((HERE.parents[2] / "build/verification").resolve()):
        parser.error("--output must be under Desktop build/verification")
    run(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
