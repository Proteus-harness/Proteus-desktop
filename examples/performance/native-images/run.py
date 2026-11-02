#!/usr/bin/env python3
"""Hold an already-built Native against deterministic images; no Cargo/Core/models."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import signal
import subprocess
import sys
import time

from fixture import ImageFixture, TRANSPORT, corpus, validate

HERE = Path(__file__).resolve().parent
STREAM = HERE.parent / "native-stream"
# Reuse the existing launch/provenance/owned-group teardown helpers.
sys.path.insert(0, str(STREAM))
spec = importlib.util.spec_from_file_location("native_images_launch_helpers", STREAM / "run.py")
helpers = importlib.util.module_from_spec(spec)
spec.loader.exec_module(helpers)
sys.path.pop(0)
save, digest = helpers.save, helpers.digest
SAMPLER = HERE.parents[3] / "Proteus-core/examples/performance/common/proc_sampler.py"


def sampler():
    spec = importlib.util.spec_from_file_location("native_images_sampler", SAMPLER)
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
                 "xdg-config", "xdg-data", "xdg-cache", "empty-extensions"):
        (args.output / name).mkdir(mode=0o700)
    manifest = corpus(args.output, args.images, args.width, args.height)
    save(args.output / "corpus.json", {"images": manifest,
         "encoded_bytes": sum(row["encoded_bytes"] for row in manifest),
         "decoded_rgba_bytes": sum(row["decoded_rgba_bytes"] for row in manifest)})
    prefs = args.output / "xdg-config/dev.proteus.native"
    prefs.mkdir(mode=0o700)
    save(prefs / "preferences.json", {"motion": "off", "preferred_agent": "proteus"})
    fixture = ImageFixture(args.output, secrets.token_hex(24), manifest,
                           image_delay_ms=args.image_delay_ms)
    env = {key: str(args.output / name) for key, name in (
        ("HOME", "home"), ("PROTEUS_HOME", "runtime"), ("PROTEUS_CONFIG_HOME", "config"),
        ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data"),
        ("XDG_CACHE_HOME", "xdg-cache"), ("PROTEUS_EXTENSIONS_DIR", "empty-extensions"))}
    env.update(SHELL="/bin/sh", PROTEUS_STARTUP_TRACE="1")
    if args.extensions:
        env["PROTEUS_EXTENSIONS_DIR"] = str(args.extensions)
    return fixture, env


def run(args):
    fixture, env = prepare(args)
    process = tree = recorder = None
    started = time.monotonic()
    interrupted = False
    state = {"status": "starting", "stage": "connecting", "native_verified": False,
             "origin": fixture.origin, "images": args.images, "width": args.width,
             "height": args.height, "decoded_rgba_bytes": args.images * args.width * args.height * 4,
             "image_delay_ms": args.image_delay_ms,
             "launcher": str(args.native), "launcher_sha256": digest(args.native),
             "source_sha256": {path.name: digest(path) for path in HERE.glob("*.py")},
             "transport_sha256": digest(TRANSPORT), "launch_helpers_sha256": digest(STREAM / "run.py"),
             "sampler_sha256": digest(SAMPLER), "marked_samples": {}}

    def record():
        with fixture.lock:
            state.update(fixture.metrics(), fixture_errors=list(fixture.errors))
            save(args.output / "trace.json", fixture.trace)
        save(args.output / "state.json", state)

    def on_signal(_signal, _frame):
        nonlocal interrupted
        interrupted = True

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
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
        while not interrupted and not (args.output / "control/stop").exists():
            if fixture.errors:
                raise RuntimeError("; ".join(fixture.errors))
            if process.poll() is not None:
                state["native_exit"] = process.returncode
                when = "before SSE connection" if state["status"] == "starting" else "without a stop request"
                raise RuntimeError(f"Native exited {when} ({process.returncode})")
            if time.monotonic() - started > args.timeout:
                raise TimeoutError("Image fixture deadline reached")
            if fixture.connected.is_set() and state["status"] == "starting":
                state.update(helpers.launched_executable(process.pid), status="running", stage="images")
                recorder.mark("images")
                print("READY · initial image snapshot delivered; GUI acceptance is separate", flush=True)
            if state["stage"] == "images" and (args.output / "control/replace").exists():
                fixture.replace()
                state["stage"] = "replaced"
                recorder.mark("replaced")
                print("STAGE replaced · empty transcript snapshot enqueued", flush=True)
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
        finally:
            if recorder:
                recorder.mark("teardown")
                recorder.sample()
                recorder.close()
                state["tree_totals"] = tree.totals()
            try:
                fixture.close()
            finally:
                state["elapsed_s"] = time.monotonic() - started
                record()
                save(args.output / "report.json", state)
                for sig, handler in previous.items():
                    signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native", type=Path, required=True, help="Ready Native executable; no builds")
    parser.add_argument("--extensions", type=Path, help="Optional ready extensions; defaults to empty")
    parser.add_argument("--images", type=int, default=16)
    parser.add_argument("--width", type=int, default=2048)
    parser.add_argument("--height", type=int, default=1024)
    parser.add_argument("--image-delay-ms", type=int, default=50,
                        help="Synthetic fetch hold, 0–250 ms; identical for before/after")
    parser.add_argument("--timeout", type=float, default=1200)
    args = parser.parse_args()
    try:
        validate(args.images, args.width, args.height)
    except ValueError as exc:
        parser.error(str(exc))
    if not 10 <= args.timeout <= 7200:
        parser.error("--timeout must be between 10 and 7200 seconds")
    if not 0 <= args.image_delay_ms <= 250:
        parser.error("--image-delay-ms must be between 0 and 250")
    args.native = args.native.resolve(strict=True)
    if not args.native.is_file() or not os.access(args.native, os.X_OK):
        parser.error("--native must be an already-built executable")
    if args.extensions:
        args.extensions = args.extensions.resolve(strict=True)
        if not args.extensions.is_dir():
            parser.error("--extensions must be a ready directory")
    args.output = args.output.resolve()
    verification = HERE.parents[2] / "build/verification"
    if not args.output.is_relative_to(verification.resolve()):
        parser.error("--output must be under Desktop build/verification")
    run(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
