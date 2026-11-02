"""Bounded synthetic PNG corpus on the shared MIT native-stream transport.

HTTP server/lifecycle is imported, not copied. See
../native-stream/LICENSE-MIT.fixture (Copyright 2026 Qweasd123tg).
"""
import copy
import importlib.util
import json
from pathlib import Path
import struct
import time
from urllib.parse import parse_qs, urlsplit
import zlib

TRANSPORT = Path(__file__).resolve().parent.parent / "native-stream/fixture_server.py"
spec = importlib.util.spec_from_file_location("native_images_transport", TRANSPORT)
base = importlib.util.module_from_spec(spec)
spec.loader.exec_module(base)
MAX_ENCODED = 5 * 1024 * 1024
MAX_DECODED = 256 * 1024 * 1024


def validate(images, width, height):
    if not 2 <= images <= 32 or images % 2:
        raise ValueError("--images must be even, between 2 and 32")
    if not 32 <= width <= 2048 or not 32 <= height <= 2048:
        raise ValueError("--width/--height must be between 32 and 2048")
    if images * width * height * 4 > MAX_DECODED:
        raise ValueError("decoded RGBA corpus exceeds 256 MiB")


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def write_png(path, width, height, index):
    """Stream scanlines into zlib; never materialize the decoded corpus in Python."""
    compressor = zlib.compressobj(6)
    compressed = bytearray()
    for y in range(height):
        # Distinct indexed checker/gradient patterns; RGBA PNG, filter type 0.
        palette = bytes(channel for x in range(256) for channel in
                        ((x * 3 + index * 29 + y // 32 * 17) % 256,
                         (x + index * 61 + y // 16 * 7) % 256,
                         (255 - x + index * 11 + y // 64 * 37) % 256, 255))
        row = (palette * ((width + 255) // 256))[:width * 4]
        compressed.extend(compressor.compress(b"\0" + row))
    compressed.extend(compressor.flush())
    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
           + chunk(b"IDAT", compressed) + chunk(b"IEND", b""))
    base.require(len(png) <= MAX_ENCODED, "Encoded image exceeds 5 MiB")
    path.write_bytes(png)
    return {"id": f"synthetic-image-{index:02d}", "path": str(path),
            "name": path.name, "mime_type": "image/png", "sha256": base.sha256(png),
            "encoded_bytes": len(png), "width": width, "height": height,
            "decoded_rgba_bytes": width * height * 4}


def corpus(output, images=16, width=2048, height=1024):
    validate(images, width, height)
    directory = Path(output) / "images"
    directory.mkdir(mode=0o700)
    return [write_png(directory / f"image-{index:02d}.png", width, height, index)
            for index in range(images)]


def transcript(manifest):
    rows = []
    for index in range(0, len(manifest), 2):
        row = base.message(f"IMAGE MEMORY · pair {index // 2 + 1}", 100 + index, role="user")
        row["images"] = [{key: image[key] for key in ("id", "name", "mime_type", "path")}
                         for image in manifest[index:index + 2]]
        rows.append(row)
        rows.append(base.message(f"AFTER IMAGE PAIR {index // 2 + 1} · scroll marker", 101 + index))
    for index, row in enumerate(rows):
        row["timestamp_ms"] = 1793059200000 + index * 60000
    return rows


class ImageFixture(base.Fixture):
    def __init__(self, output, token, manifest, *, image_delay_ms=50):
        if not 0 <= image_delay_ms <= 250:
            raise ValueError("image_delay_ms must be between 0 and 250")
        self.image_delay_ms = image_delay_ms
        self.manifest = copy.deepcopy(manifest)
        self.images = {image["id"]: Path(image["path"]) for image in manifest}
        super().__init__(Path(output) / "workspace", Path(output) / "session", token,
                         transcript(manifest))
        self.initial = copy.deepcopy(self.transcript)
        self.image_requests = 0
        self.active_images = 0
        self.max_active_images = 0
        self.image_ids = {}
        self.replaced = False
        self.snapshot_emissions = 0

    def emit(self, event):
        if event["type"] != "session_snapshot":
            raise ValueError("Image fixture accepts snapshots only")
        snapshot = event["snapshot"]
        base.require(snapshot["session_id"] == base.SESSION_ID and
                     snapshot["root_thread_id"] == base.THREAD_ID and
                     snapshot["stream_id"] == "native-fixture-incarnation" and
                     snapshot["seq"] == self.seq == self.snapshot_emissions,
                     "Invalid snapshot identity/cursor")
        expected = [] if self.replaced else self.initial
        base.require(snapshot["transcript"] == expected, "Snapshot differs from immutable fixture stage")
        base.require(not self.stopping.is_set(), "Fixture stopped")
        data = json.dumps({"type": "event", "event": event},
                          ensure_ascii=False, separators=(",", ":")).encode()
        base.require(len(data) <= 1024 * 1024, "Snapshot exceeds SSE bound")
        row = self.record("event_enqueued", event_type="session_snapshot",
                          snapshot_seq=self.seq, payload_sha256=base.sha256(data))
        self.pending.put_nowait((data, row["trace_seq"]))
        self.snapshot_emissions += 1
        return row["at_ns"]

    def replace(self):
        with self.lock:
            base.require(self.connected.is_set() and not self.replaced,
                         "Replacement requires connected initial stage and is allowed once")
            self.seq += 1
            self.replaced = True
            self.transcript = []
            return self.emit({"type": "session_snapshot", "snapshot": {
                "session_id": base.SESSION_ID, "stream_id": "native-fixture-incarnation",
                "seq": self.seq, "root_thread_id": base.THREAD_ID, "transcript": [],
                "history": None, "execution": {"active": None, "last": None}}})

    def metrics(self):
        with self.lock:
            return {"image_requests": self.image_requests,
                    "active_image_requests": self.active_images,
                    "max_active_image_requests": self.max_active_images,
                    "image_ids": dict(self.image_ids), "snapshot_seq": self.seq,
                    "replaced": self.replaced, "request_count": len(self.requests)}

    def handle(self, h, method):
        parsed = urlsplit(h.path)
        if parsed.path not in ("/image", "/model/quota", "/history/tool-state", "/sessions/current"):
            return super().handle(h, method)
        h.connection.settimeout(15)
        if h.headers.get("Authorization") != "Bearer " + self.token:
            h.reply(401, {"error": "Invalid synthetic token"})
            return
        query = parse_qs(parsed.query, strict_parsing=True, keep_blank_values=True)
        self.record("http_received", method=method, path=parsed.path, query=query)
        allowed = ({"session_dir", "path"} if parsed.path == "/image" else
                   {"session_dir", "name"} if parsed.path == "/history/tool-state" else {"session_dir"})
        base.require(method == "GET" and not parsed.fragment and set(query) == allowed
                     and all(len(value) == 1 for value in query.values())
                     and query["session_dir"] == [self.session], "Invalid scoped fixture query")
        with self.lock:
            base.require(len(self.requests) < 4096, "Fixture request cap exceeded")
            self.requests.append(self.record("http_request", method=method, path=parsed.path,
                                             session=self.session))
        if parsed.path == "/model/quota":
            h.reply(200, None)
        elif parsed.path == "/sessions/current":
            h.reply(200, self.session_summaries())
        elif parsed.path == "/history/tool-state":
            base.require(query["name"] == ["update_plan"], "Unknown tool-state query")
            h.reply(200, {"revision": "image-fixture", "tool": None})
        else:
            image_id = query["path"][0]
            image = self.images.get(image_id)
            if image is None:
                h.reply(404, {"error": "Unknown synthetic image ID"})
                return
            with self.lock:
                self.image_requests += 1
                self.image_ids[image_id] = self.image_ids.get(image_id, 0) + 1
                self.active_images += 1
                self.max_active_images = max(self.max_active_images, self.active_images)
            try:
                time.sleep(self.image_delay_ms / 1000)
                base.require(image.stat().st_size <= MAX_ENCODED, "Encoded file exceeds 5 MiB")
                data = image.read_bytes()  # Only the requested encoded PNG is resident.
                base.require(len(data) <= MAX_ENCODED, "Encoded response exceeds 5 MiB")
                h.send_response(200)
                h.send_header("Content-Type", "image/png")
                h.send_header("Content-Length", str(len(data)))
                h.send_header("Connection", "close")
                h.end_headers()
                h.wfile.write(data)
                h.wfile.flush()
                h.close_connection = True
            finally:
                with self.lock:
                    self.active_images -= 1
