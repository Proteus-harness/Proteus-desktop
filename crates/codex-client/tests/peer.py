#!/usr/bin/env python3
"""Synthetic account-free app-server peer; never reads user Codex state."""
import json
import os
import sys
import time


def emit(message):
    print(json.dumps(message, separators=(",", ":")), flush=True)


def reply(request, value):
    emit({"id": request["id"], "result": value})


held = []
approval = None
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        assert request["params"]["clientInfo"]["name"] == "proteus-desktop"
        assert request["params"]["capabilities"]["experimentalApi"] is True
        reply(request, {"userAgent": "synthetic-peer"})
    elif method == "initialized":
        pass
    elif method == "echo":
        reply(request, request["params"])
    elif method == "reverse":
        held.append(request)
        if len(held) == 2:
            for item in reversed(held):
                reply(item, item["params"])
            held.clear()
    elif method == "approval":
        approval = request
        emit({"method": "turn/started", "params": {"threadId": "synthetic"}})
        emit({"id": request["params"]["serverId"], "method": "item/commandExecution/requestApproval", "params": {"command": "true"}})
    elif method == "turn/interrupt":
        reply(request, {"interrupted": True})
    elif method is None:
        assert approval is not None
        assert request["id"] == approval["params"]["serverId"]
        reply(approval, request.get("result", request.get("error")))
        approval = None
    elif method == "remote_error":
        emit({"id": request["id"], "error": {"code": -123, "message": "synthetic error", "data": {"retry": False}}})
    elif method == "eof":
        sys.exit(0)
    elif method == "oversize":
        sys.stdout.write("x" * 65536)
        sys.stdout.flush()
        time.sleep(10)
    elif method == "malformed":
        print("secret fixture body is not valid JSON", flush=True)
    elif method == "flood":
        for index in range(200):
            emit({"method": "delta", "params": {"index": index, "text": "x" * 200}})
    elif method == "fill_then_reply":
        emit({"method": "delta", "params": {"text": "x" * 200}})
        reply(request, "y" * 200)
    elif method == "timeout":
        emit({"method": "holding", "params": {}})
    elif method == "late":
        time.sleep(0.05)
        reply(request, "late result")
    elif method == "close_stdin":
        reply(request, None)
        os.close(0)
        emit({"method": "stdin_closed", "params": {}})
        time.sleep(10)
    elif method == "stop_reading":
        reply(request, None)
        time.sleep(10)
    elif method == "pid":
        reply(request, os.getpid())
    else:
        raise AssertionError("unexpected fixture command")
