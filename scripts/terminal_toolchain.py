#!/usr/bin/env python3
"""Prepare the pinned, build-only Ghostty toolchain without touching system tools.

The public entry point ``prepare(root, environ)`` returns the environment for
Cargo (including absolute ZIG). Runtime portable archives do not include these
files. Existing Cargo/Zig caches are retained.
"""
from __future__ import annotations

import argparse
import hashlib
import fcntl
import json
import os
import platform
import re
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import tarfile
import urllib.request

ZIG_VERSION = "0.17.0"
ZIG_URL = "https://ziglang.org/download/0.17.0/zig-x86_64-linux-0.17.0.tar.xz"
ZIG_SHA256 = "1cbe9df9f27e6b78d14ccbca43b6703a404ef79ef1c463de901d7f088d4e2026"
# Exact upstream revisions and Zig package content hashes from Ghostty 83fb956.
# Primary project archives avoid the deps.files.ghostty.org mirror, which may
# reject requests from some build environments. Hashes remain unchanged.
PACKAGES = [
    ("uucode", "https://codeload.github.com/jacobsandlund/uucode/tar.gz/ea62149739404a73c202b48a33bf6dd2af4bd9b0",
     "uucode-0.2.0-ZZjBPoGFVgCiflJNFc6Io8xzC67B6PA5bUkk4LT0kCxg"),
    ("translate-c", "https://codeberg.org/ziglang/translate-c/archive/02ff0c523fb92a38939b04d0145c4a9602a74d56.tar.gz",
     "translate_c-2.0.0-Q_BUWltOBwBYABF84EUL208pIFeUIwbBs29FmVBaafN5"),
    ("aro", "https://codeberg.org/ziglang/arocc/archive/d0c8c4d9c55daa7ef6e40cf0f630a5b5e900989b.tar.gz",
     "aro-0.0.0-JSD1QtuBNwCASyBtNF3pqTl_W3oAJQGEVyFAtrBSE_Pa"),
    ('highway', 'https://codeload.github.com/google/highway/tar.gz/66486a10623fa0d72fe91260f96c892e41aceb06',
     'N-V-__8AAGmZhABbsPJLfbqrh6JTHsXhY6qCaLAQyx25e0XE'),
    ('wuffs', 'https://codeload.github.com/google/wuffs-mirror-release-c/tar.gz/7411f488fe2e2c205c3d3b3d28638b7356522930',
     'N-V-__8AAP5JWgCGP_AD0teWpa4krRvE9VPZzvviGdbmN4jI'),
    ('zlib', 'https://codeload.github.com/madler/zlib/tar.gz/refs/tags/v1.3.1',
     'N-V-__8AAB0eQwD-0MdOEBmz7intriBReIsIDNlukNVoNu6o'),
    ('pixels', 'https://codeload.github.com/make-github-pseudonymous-again/pixels/tar.gz/d843c2714d32e15b48b8d7eeb480295af537f877',
     'N-V-__8AADYiAAB_80AWnH1AxXC0tql9thT-R-DYO1gBqTLc'),
]


def download(url: str, destination: Path, sha256: str | None = None) -> str:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.exists():
        temporary = destination.with_name(destination.name + ".download")
        with urllib.request.urlopen(url, timeout=60) as response, temporary.open("wb") as output:
            shutil.copyfileobj(response, output)
        digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
        if sha256 and digest != sha256:
            temporary.unlink()
            raise RuntimeError(f"{url}: SHA256 mismatch: {digest}")
        temporary.replace(destination)
    digest = hashlib.sha256(destination.read_bytes()).hexdigest()
    if sha256 and digest != sha256:
        raise RuntimeError(f"{destination}: SHA256 mismatch: {digest}")
    return digest


def flatten_archive(source: Path, destination: Path) -> None:
    """Strip the archive's one enclosing directory, preserving package content.

    Zig 0.17's local-tar fetch otherwise writes two enclosing directories into
    its cache. The first fetch prints the right content hash but later builds
    fail the same hash on the malformed cached package. A flat input archive
    yields the ordinary one-directory Zig cache layout.
    """
    if destination.exists():
        return
    temporary = destination.with_name(destination.name + ".partial")
    prefix = None
    with tarfile.open(source) as archive, tarfile.open(temporary, "w:gz") as output:
        for member in archive:
            if member.type in (tarfile.XHDTYPE, tarfile.XGLTYPE):
                continue
            parts = PurePosixPath(member.name).parts
            if not parts or ".." in parts or member.name.startswith("/"):
                raise RuntimeError(f"unsafe source archive entry: {member.name}")
            if prefix is None:
                prefix = parts[0]
            if parts[0] != prefix:
                raise RuntimeError(f"source archive has multiple roots: {source}")
            if len(parts) == 1:
                continue
            contents = archive.extractfile(member) if member.isfile() else None
            member.name = "/".join(parts[1:])
            output.addfile(member, contents)
    temporary.replace(destination)


def _version(zig: str, environ: dict[str, str]) -> str:
    return subprocess.run([zig, "version"], env=environ, check=True,
                          capture_output=True, text=True).stdout.strip()


def fingerprint(path: Path) -> list[int]:
    status = path.stat()
    return [status.st_size, status.st_mtime_ns]


def receipt_valid(receipt: dict, specification: dict) -> bool:
    if receipt.get("specification") != specification or not receipt.get("files"):
        return False
    try:
        return all(fingerprint(Path(path)) == status for path, status in receipt["files"].items())
    except (OSError, TypeError, ValueError):
        return False


def _prepare_locked(root: Path | None = None, environ: dict[str, str] | None = None) -> dict[str, str]:
    root = (root or Path(__file__).resolve().parents[1]).resolve()
    environment = dict(os.environ if environ is None else environ)
    tools = root / "build/tools"
    explicit = environment.get("ZIG")
    zig = explicit or shutil.which("zig", path=environment.get("PATH"))
    if explicit and _version(explicit, environment) != ZIG_VERSION:
        raise RuntimeError(f"ZIG must be version {ZIG_VERSION}: {explicit}")
    if zig and _version(zig, environment) != ZIG_VERSION:
        zig = None
    if not zig:
        if platform.system() != "Linux" or platform.machine() not in ("x86_64", "AMD64"):
            raise RuntimeError(f"Install Zig {ZIG_VERSION} and set ZIG for this platform")
        archive = tools / "zig-x86_64-linux-0.17.0.tar.xz"
        download(ZIG_URL, archive, ZIG_SHA256)
        binary = tools / "zig-x86_64-linux-0.17.0/zig"
        if not binary.is_file():
            with tarfile.open(archive) as source:
                source.extractall(tools, filter="data")
        zig = str(binary)
    zig = str(Path(shutil.which(zig, path=environment.get("PATH")) or zig).resolve())
    if _version(zig, environment) != ZIG_VERSION:
        raise RuntimeError(f"Expected Zig {ZIG_VERSION}: {zig}")
    environment["ZIG"] = zig
    report = tools / "ghostty-toolchain.json"
    specification = {"zig": zig, "version": ZIG_VERSION, "packages": PACKAGES,
        "cache_context": {key: environment.get(key) for key in ("ZIG_GLOBAL_CACHE_DIR", "ZIG_LOCAL_PKG_DIR", "XDG_CACHE_HOME", "HOME")}}
    # JSON represents package tuples as lists; compare canonical JSON shapes.
    specification = json.loads(json.dumps(specification))
    try:
        if receipt_valid(json.loads(report.read_text()), specification):
            return environment
    except (OSError, ValueError, TypeError):
        pass
    zig_env = subprocess.run([zig, "env"], env=environment, check=True, capture_output=True, text=True).stdout
    cache_match = re.search(r'\.global_cache_dir\s*=\s*("(?:[^"\\]|\\.)*")', zig_env)
    if not cache_match:
        raise RuntimeError("Zig did not report its global cache directory")
    cache = Path(json.loads(cache_match.group(1))) / "p"
    files = {zig: fingerprint(Path(zig))}
    packages = []
    for name, url, expected in PACKAGES:
        archive = tools / "ghostty-deps" / f"{name}.tar.gz"
        digest = download(url, archive)
        flat = archive.with_name(f"{name}-flat.tar.gz")
        flatten_archive(archive, flat)
        fetched = subprocess.run([zig, "fetch", str(flat)], env=environment,
                                 check=True, capture_output=True, text=True)
        actual = fetched.stdout.strip()
        if actual != expected:
            raise RuntimeError(f"{name}: expected Zig hash {expected}, got {actual}")
        packages.append({"name": name, "url": url, "sha256": digest, "zig_hash": actual})
        for path in (archive, flat, cache / (expected + ".tar.gz")):
            files[str(path)] = fingerprint(path)

    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(json.dumps({"zig": zig, "version": ZIG_VERSION,
        "official_zig_sha256": ZIG_SHA256, "packages": packages,
        "specification": specification, "files": files}, indent=2) + "\n")
    return environment


def prepare(root: Path | None = None, environ: dict[str, str] | None = None) -> dict[str, str]:
    root = (root or Path(__file__).resolve().parents[1]).resolve()
    tools = root / "build/tools"
    tools.mkdir(parents=True, exist_ok=True)
    # run/test/portable may prepare before Cargo acquires its artifact lock.
    # Serialize the shared download, normalization and receipt paths.
    with (tools / ".ghostty-setup.lock").open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        return _prepare_locked(root, environ)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path)
    args = parser.parse_args()
    environment = prepare(args.root)
    print(json.dumps({"ZIG": environment["ZIG"]}))


if __name__ == "__main__":
    main()
