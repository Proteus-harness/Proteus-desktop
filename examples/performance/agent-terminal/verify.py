#!/usr/bin/env python3
"""Exercise the bundled Core + shell-tool through an owned synthetic Desktop.

Never runs sudo; requires the isolated runner's output and its Desktop PID.
The generated test password stays in memory and is never printed or saved.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import secrets
import time
import urllib.parse
import urllib.request


def descendants(pid):
    pending = [pid]
    seen = set()
    while pending:
        current = pending.pop()
        if current in seen:
            continue
        seen.add(current)
        yield current
        for children in Path(f"/proc/{current}/task").glob("*/children"):
            try:
                pending.extend(map(int, children.read_text().split()))
            except (FileNotFoundError, PermissionError):
                pass


def connection(root, desktop_pid):
    profile = str(root / "config/terminal-demo.toml")
    matches = []
    for pid in descendants(desktop_pid):
        try:
            args = Path(f"/proc/{pid}/cmdline").read_bytes().decode().strip("\0").split("\0")
        except (FileNotFoundError, PermissionError):
            continue
        if profile in args and all(value in args for value in ("server", "http", "--token")):
            matches.append((pid, args))
    assert len(matches) == 1, "expected one Core owned by the synthetic Desktop"
    pid, args = matches[0]
    inodes = set()
    for fd in Path(f"/proc/{pid}/fd").iterdir():
        try:
            target = os.readlink(fd)
        except FileNotFoundError:
            continue
        if target.startswith("socket:["):
            inodes.add(target[8:-1])
    ports = []
    for line in Path(f"/proc/{pid}/net/tcp").read_text().splitlines()[1:]:
        fields = line.split()
        if fields[3] == "0A" and fields[9] in inodes:
            address, port = fields[1].split(":")
            assert address == "0100007F", "fixture Core must listen on loopback"
            ports.append(int(port, 16))
    assert len(ports) == 1, "expected one owned Core listener"
    return f"http://127.0.0.1:{ports[0]}", args[args.index("--token") + 1]


def objects(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from objects(child)
    elif isinstance(value, list):
        for child in value:
            yield from objects(child)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--desktop-pid", required=True, type=int)
    args = parser.parse_args()
    root = args.output.resolve(strict=True)
    assert (root / "prepared.json").is_file(), "use run.py's isolated output"
    origin, token = connection(root, args.desktop_pid)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def request(path, query=None, body=None):
        url = origin + path + ("?" + urllib.parse.urlencode(query) if query else "")
        headers = {"Authorization": "Bearer " + token}
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(body).encode()
        with opener.open(urllib.request.Request(url, data, headers), timeout=10) as response:
            return json.load(response)

    bootstrap = request("/bootstrap")
    assert Path(bootstrap["cwd"]) == root / "workspace", "wrong fixture workspace"
    session = bootstrap["session_dir"]
    assert session and Path(session).is_relative_to(root), "wrong fixture session"
    query = {"session_dir": session}
    existing = {obj["terminal"]["id"] for obj in objects(request("/history", query))
                if isinstance(obj.get("terminal"), dict)}
    started = time.monotonic()
    result = request("/send-async", body={"id": "terminal-wire-" + secrets.token_hex(8),
                                        "text": "Synthetic terminal wire check", **query})
    assert not result.get("error"), "fixture send failed"
    terminal = None
    while time.monotonic() - started < 10:
        for obj in objects(request("/history", query)):
            ref = obj.get("terminal")
            if isinstance(ref, dict) and ref["id"] not in existing:
                assert obj.get("result") is None, "terminal was published only after tool yield"
                terminal = ref
                break
        if terminal:
            break
        time.sleep(0.1)
    assert terminal and terminal["tty"], "no early interactive terminal reference"
    early_seconds = time.monotonic() - started
    address = dict(query, id=terminal["id"])
    cursor = None
    raw = bytearray()

    def read():
        nonlocal cursor
        params = dict(address, max_bytes=65536, wait_ms=500)
        if cursor is not None:
            params["cursor"] = cursor
        response = request("/terminals/read", params)
        assert response["type"] == "read", "wrong terminal read response"
        snapshot = response["snapshot"]
        assert not snapshot["gap"], "unexpected gap in tiny fixture"
        raw.extend(base64.b64decode(snapshot["data_base64"], validate=True))
        cursor = snapshot["next_cursor"]
        return snapshot

    deadline = time.monotonic() + 10
    while "Пароль:".encode() not in raw and time.monotonic() < deadline:
        read()
    assert "Пароль:".encode() in raw, "missing hidden-input prompt"
    assert request("/terminals/resize", body=dict(address, columns=100, rows=30))["type"] == "accepted"
    password = ("synthetic-" + secrets.token_hex(16)).encode()
    # Deliberately a single request: an uncertain write is never retried.
    assert request("/terminals/input", body=dict(address,
            data_base64=base64.b64encode(password + b"\r").decode()))["type"] == "accepted"
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        snapshot = read()
        if snapshot["finished"] and snapshot["next_cursor"] == snapshot["end_cursor"]:
            break
    else:
        raise AssertionError("terminal did not exit and drain")
    assert snapshot["exit_code"] == 0, "fixture command failed"
    assert b"PASSWORD_ACCEPTED" in raw and b"TERMINAL_DEMO_DONE" in raw, "missing final output"
    assert password not in raw, "password echoed into terminal output"
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        history = request("/history", query)
        if any(obj.get("terminal", {}).get("id") == terminal["id"] and obj.get("result")
               for obj in objects(history) if isinstance(obj.get("terminal"), dict)):
            break
        time.sleep(0.1)
    else:
        raise AssertionError("tool did not settle")
    for path in ("/history", "/conversation"):
        assert password not in json.dumps(request(path, query)).encode(), "password in model history"
    checked = 0
    for path in Path(session).rglob("*"):
        if path.is_file():
            assert password not in path.read_bytes(), "password in session storage"
            checked += 1
    receipt = {"passed": True, "origin": origin, "terminal": terminal["id"],
               "early_reference_seconds": round(early_seconds, 3), "exit_code": 0,
               "output_bytes": len(raw), "session_files_checked": checked,
               "checks": ["early_reference_before_yield", "real_module_pty", "resize",
                          "hidden_input", "independent_output", "final_drain",
                          "no_password_in_output_history_or_storage"]}
    destination = root / "wire-receipt.json"
    destination.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt, indent=2))


if __name__ == "__main__":
    main()
