#!/usr/bin/env python3
"""Hold synthetic code fences in a ready Native for geometry and exact Copy checks."""
import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

from fixture_server import Fixture, MESSAGE_ID, SESSION_ID, THREAD_ID, TURN_ID, message
from run import digest, launched_executable, stop_native
from status_demo import DIRECTORIES, delivered, record

SOURCES = {
    "rust": ('let emoji = "☕ 🚀";\nprintln!("{emoji}");\n', "rust"),
    "text": ("short plain text\n", "text"),
    "markdown": ("**short markdown**\n", "md"),
    # The CJK row has more UTF-8 bytes, but fewer monospace cells than the
    # ASCII row. Tabs and emoji exercise actual shaping rather than len().
    "unicode": ("// " + "a" * 56 + "\n// " + "界" * 22
                + '\n\t\tlet emoji = "☕ 🚀";\n', "rust"),
    "long": ('let payload = "' + "0123456789" * 20 + '";\n', "rust"),
}


def sources(stage):
    result = dict(SOURCES)
    if stage == "updated":
        result["rust"] = (SOURCES["rust"][0] + 'println!("updated");\n', "rust")
    return result


def text(stage):
    return "CODE BLOCK FIXTURE · " + stage + "\n\n" + "\n".join(
        "```" + language + "\n" + source + "```\n"
        for source, language in sources(stage).values())


def transcript():
    row = message(text("initial"), int(MESSAGE_ID.replace("-", ""), 16))
    row["timestamp_ms"] = 1793059200000
    assert row["message_id"] == MESSAGE_ID
    return [row]


def update(fixture):
    fixture.run_id = "run-synthetic-code-blocks"
    fixture.execution("running")
    fixture.runtime("TurnStarted", {"session_id": SESSION_ID,
                                   "thread_id": THREAD_ID, "turn_id": TURN_ID})
    # Replace the same message explicitly, with closed fences on both sides.
    # A tool barrier exposes Copy without a second snapshot or completion.
    fixture.delta(0, text("updated"))
    fixture.tool("codeblockfreeze")
    fixture.execution("success", terminal=True)


def expected(output):
    for stage in ("initial", "updated"):
        directory = output / "expected" / stage
        directory.mkdir(parents=True, mode=0o700)
        for name, (source, _language) in sources(stage).items():
            (directory / (name + ".txt")).write_bytes(source.encode())
        (directory / "message.txt").write_bytes(text(stage).encode())


def select_renderer(output, installed):
    """Expose only the ready Markdown package; unrelated badges stay unloaded."""
    source = (installed / "markdown-render").resolve(strict=True)
    manifest_path = source / "extension.json"
    manifest = json.loads(manifest_path.read_text())
    if (manifest.get("id") != "markdown-render"
            or "text/markdown" not in manifest.get("surfaces", {}).get("content", {}).get("formats", [])):
        raise ValueError("--extensions must contain the installed markdown-render content package")
    command = manifest.get("command", [])
    if not command or not isinstance(command[0], str):
        raise ValueError("markdown-render has no executable command")
    executable = source / command[0]
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError("markdown-render must already be built and executable")
    selected = output / "renderer-extensions"
    selected.mkdir(mode=0o700)
    (selected / "markdown-render").symlink_to(source, target_is_directory=True)
    return {"selected_extensions": str(selected), "renderer": str(source),
            "renderer_manifest_sha256": digest(manifest_path),
            "renderer_executable_sha256": digest(executable)}


def check_fixture(output):
    """Wire/source oracle only; never launches Native or claims GUI acceptance."""
    fixture = Fixture(output / "workspace", output / "session",
                      secrets.token_hex(24), transcript()).start()
    try:
        client = build_opener(ProxyHandler({}))
        request = Request(fixture.origin + "/events?" + urlencode({"session_dir": fixture.session}),
                          headers={"Authorization": "Bearer " + fixture.token})
        with client.open(request, timeout=3) as stream:
            def read():
                while True:
                    line = stream.readline()
                    if not line:
                        raise RuntimeError("Fixture SSE closed early")
                    if line.startswith(b"data: "):
                        return json.loads(line[6:])["event"]

            snapshot = read()["snapshot"]
            assert snapshot["transcript"] == transcript()
            assert snapshot["transcript"][0]["streaming"] is False
            update(fixture)
            running, turn, delta, barrier, done = (read() for _ in range(5))
            assert running["execution"]["active"]["status"] == "running"
            assert turn["envelope"]["event"]["TurnStarted"]["thread_id"] == THREAD_ID
            body = delta["envelope"]["event"]["AssistantTextDelta"]
            assert body["message_id"] == MESSAGE_ID and body["offset"] == 0
            assert body["text"] == (output / "expected/updated/message.txt").read_text()
            assert "ToolCallRequested" in barrier["envelope"]["event"]
            assert done["execution"]["active"] is None
            assert done["execution"]["last"]["status"] == "success"
            delivered(fixture)
            fixture.stopping.set()
        for stage in ("initial", "updated"):
            for name, (source, _language) in sources(stage).items():
                assert (output / "expected" / stage / (name + ".txt")).read_bytes() == source.encode()
        rows = SOURCES["unicode"][0].splitlines()
        assert len(rows[1].encode()) > len(rows[0].encode())
        assert len(SOURCES["long"][0].rstrip("\n")) > 180
        record(output, fixture, {"status": "fixture-smoke-passed", "rows": 1,
                                 "stages": ["initial", "updated"], "native_verified": False})
        print("PASS fixture: one message, five fences, explicit revision; no GUI checked", flush=True)
    finally:
        fixture.close()


def run(args):
    fixture = Fixture(args.output / "workspace", args.output / "session",
                      secrets.token_hex(24), transcript()).start()
    process = None
    started = time.monotonic()
    interrupted = False
    state = {"status": "starting", "stage": "connecting", "rows": 1,
             "origin": fixture.origin, "timeout_s": args.timeout,
             "launcher": str(args.native), "launcher_sha256": digest(args.native),
             "extensions": str(args.extensions), "expected_dir": str(args.output / "expected"),
             **args.renderer,
             "source_sha256": {name: digest(Path(__file__).with_name(name)) for name in
                               ("code_blocks.py", "status_demo.py", "fixture_server.py", "run.py")}}

    def on_signal(_signum, _frame):
        nonlocal interrupted
        interrupted = True

    previous = {sig: signal.signal(sig, on_signal) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        env = os.environ.copy()
        env.update({key: str(args.output / directory) for key, directory in (
            ("HOME", "home"), ("PROTEUS_HOME", "runtime"), ("PROTEUS_CONFIG_HOME", "config"),
            ("XDG_CONFIG_HOME", "xdg-config"), ("XDG_DATA_HOME", "xdg-data"),
            ("XDG_CACHE_HOME", "xdg-cache"))})
        env["PROTEUS_EXTENSIONS_DIR"] = state["selected_extensions"]
        with (args.output / "native.log").open("w") as log:
            process = subprocess.Popen([str(args.native), str(args.output / "workspace"),
                                        "--origin", fixture.origin, "--token", fixture.token],
                                       env=env, stdout=log, stderr=log, start_new_session=True)
            state["pid"] = process.pid
            record(args.output, fixture, state)
            while not interrupted:
                if fixture.errors:
                    raise RuntimeError("; ".join(fixture.errors))
                if process.poll() is not None:
                    raise RuntimeError(f"Native exited early ({process.returncode}); see native.log")
                if time.monotonic() - started > args.timeout:
                    raise TimeoutError("Code block demo deadline reached")
                if (args.output / "control/stop").exists():
                    break
                if fixture.connected.is_set() and state["status"] == "starting":
                    state.update(launched_executable(process.pid))
                    delivered(fixture)
                    state.update(status="running", stage="initial")
                    record(args.output, fixture, state)
                    print("READY " + json.dumps(state), flush=True)
                if state["stage"] == "initial" and (args.output / "control/update").exists():
                    update(fixture)
                    delivered(fixture)
                    state["stage"] = "updated"
                    record(args.output, fixture, state)
                    print("STAGE updated", flush=True)
                time.sleep(.05)
        state["status"] = "stopped"
    except BaseException as exc:
        state.update(status="failed", error=type(exc).__name__ + ": " + str(exc))
        raise
    finally:
        try:
            fixture.stopping.set()
            if process is not None:
                stop_native(process)
        finally:
            try:
                fixture.close()
            finally:
                state.update(fixture_errors=fixture.errors, elapsed_s=time.monotonic() - started)
                record(args.output, fixture, state)
                for sig, handler in previous.items():
                    signal.signal(sig, handler)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--native", type=Path)
    parser.add_argument("--extensions", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--check-fixture", action="store_true")
    args = parser.parse_args()
    if not 10 <= args.timeout <= 600:
        parser.error("--timeout must be between 10 and 600 seconds")
    if not args.check_fixture:
        if args.native is None or args.extensions is None:
            parser.error("--native READY and --extensions INSTALLED_DIR are required")
        args.native = args.native.resolve(strict=True)
        args.extensions = args.extensions.resolve(strict=True)
        if not os.access(args.native, os.X_OK) or not args.native.is_file():
            parser.error("--native must be an already-built executable")
        if not args.extensions.is_dir():
            parser.error("--extensions must be an installed extensions directory")
    args.output = args.output.resolve()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    for directory in DIRECTORIES:
        (args.output / directory).mkdir(mode=0o700)
    expected(args.output)
    if args.check_fixture:
        check_fixture(args.output)
    else:
        args.renderer = select_renderer(args.output, args.extensions)
        run(args)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, ValueError, TimeoutError, AssertionError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr, flush=True)
        sys.exit(1)
