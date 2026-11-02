#!/usr/bin/env python3
"""Minimal real protocol-v4 content contributor. Never stores session credentials."""
import json
from pathlib import Path
import sys

from corpus import HEIGHT, LANGUAGE, WIDTH, STAGES, hashes, markdown, stage_from_source, svgs

MAX_LINE = 1024 * 1024
MAX_RECEIPTS = 512


def response(message):
    content = message["content"]
    if message.get("surface") != "content":
        raise ValueError("Unexpected synthetic renderer surface")
    language = content.get("language")
    if language == LANGUAGE:
        stage = stage_from_source(content["source"])
    elif language == "text/markdown":
        matches = [stage for stage in range(STAGES) if content["source"] == markdown(stage)]
        if not matches:
            raise ValueError("Markdown source is not an exact synthetic fence")
        stage = matches[0]
    else:
        raise ValueError("Unexpected synthetic renderer language")
    root = {"type": "stack", "direction": "column", "children": [
        {"type": "svg", "source": svg, "width": WIDTH, "height": HEIGHT} for svg in svgs(stage)]}
    return ({"type": "view", "view": message["view"], "session_dir": message.get("session_dir"),
             "sequence": 0, "source_revision": content.get("source_revision"), "root": root},
            {"stage": stage, "language": language, "view": message["view"], "source_revision": content.get("source_revision"),
             "svg_sha256": hashes(stage)})


def run(receipts):
    count = 0
    with Path(receipts).open("a", buffering=1) as log:
        while True:
            line = sys.stdin.buffer.readline(MAX_LINE + 1)
            if not line:
                break
            if len(line) > MAX_LINE or not line.endswith(b"\n"):
                raise ValueError("Renderer input exceeds bounded JSON line")
            message = json.loads(line)
            kind = message["type"]
            if kind == "hello":
                if message["protocol"] != 4:
                    raise ValueError("Expected protocol 4")
            elif kind == "render":
                count += 1
                if count > MAX_RECEIPTS:
                    raise ValueError("Renderer receipt bound exceeded")
                reply, receipt = response(message)
                # Receipt confirms callback output, never claims a successful paint.
                print(json.dumps(reply, separators=(",", ":")), flush=True)
                log.write(json.dumps(receipt, separators=(",", ":")) + "\n")
            elif kind not in ("context", "close", "close_context", "event", "rehost", "config"):
                raise ValueError("Unexpected renderer input type: " + kind)


if __name__ == "__main__":
    run(sys.argv[1])
