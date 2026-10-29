"""Ready-package validation, provenance and transactional publication; no builds."""
import hashlib
import json
import os
import re
import shutil
from pathlib import Path

COMPONENTS = ("desktop", "core", "modules", "extensions")
SOURCE_NAMES = dict(zip(COMPONENTS, (
    "Proteus-desktop", "Proteus-core", "Proteus-modules", "Proteus-desktop-extensions",
)))


def read_record(base):
    try:
        record = json.loads((base / "share/release-sources.json").read_text())
        required = ("bin/proteus", "bin/proteus-native", "proteus-native",
                    "install-shortcut", "share/system_proxy.py")
        if any(not (base / name).is_file() for name in required):
            raise ValueError("missing runtime or launcher")
        for name in ("bin/proteus", "bin/proteus-native", "proteus-native", "install-shortcut"):
            if not os.access(base / name, os.X_OK):
                raise ValueError(f"not executable: {name}")
        if any(not (base / "share" / name).is_dir()
               for name in ("configs", "modules", "extensions", "licenses", "icons")):
            raise ValueError("missing runtime assets")
        for name in SOURCE_NAMES.values():
            if not isinstance(record["sources"][name]["commit"], str):
                raise ValueError("missing source provenance")
            if not (base / "share/licenses" / name).is_dir():
                raise ValueError(f"missing notices for {name}")
        if not isinstance(record["version"], str):
            raise ValueError("missing version")
        manifests = list((base / "share/extensions").glob("*/extension.json"))
        modules = list((base / "share/modules").glob("*/module.json"))
        if not manifests or not modules or not any((base / "share/configs").rglob("*")):
            raise ValueError("empty runtime assets")
        for path in manifests + modules:
            spec = json.loads(path.read_text())
            program = Path(spec["command"][0])
            root = path.parent if path.name == "extension.json" else base
            if program.is_absolute() or ".." in program.parts or not (root / program).is_file() or not os.access(root / program, os.X_OK):
                raise ValueError(f"missing executable for {path}")
        extension_protocols(base)
        return record
    except (OSError, ValueError, KeyError, TypeError, IndexError) as error:
        raise SystemExit(
            f"No complete ready portable at {base}: {error}. "
            "Use --base PATH with a prebuilt portable, or --full for an explicit bootstrap."
        ) from error


def extension_protocols(base):
    return {json.loads(path.read_text())["protocol"]
            for path in (base / "share/extensions").glob("*/extension.json")}


def desktop_protocol(native):
    source = (native / "src/declarative/protocol.rs").read_text()
    match = re.search(r"pub const PROTOCOL: u32 = (\d+);", source)
    if not match:
        raise SystemExit("Cannot determine Desktop extension protocol")
    return int(match[1])


def verify_protocol(base, native, selected, record):
    if "desktop" in selected and "extensions" not in selected:
        if extension_protocols(base) != {desktop_protocol(native)}:
            raise SystemExit("Ready extensions use another protocol; explicitly select "
                             "--components desktop extensions (no components are rebuilt automatically)")
    # The selected Extensions protocol comes from its own source checkout,
    # not an unrelated, unselected Desktop checkout.


def verify_extension_source(base, extensions, record):
    expected = record.get("desktop_protocol")
    if expected is None:
        protocols = extension_protocols(base)
        expected = next(iter(protocols)) if len(protocols) == 1 else None
    protocols = {json.loads(path.read_text())["protocol"]
                 for path in (extensions / "packages").glob("*/extension.json")}
    if protocols != {expected}:
        raise SystemExit("Selected extensions use another protocol than the ready Desktop; "
                         "explicitly select --components desktop extensions")


def binary_hashes(stage):
    result = {}
    for binary in sorted((stage / "bin").iterdir()):
        if binary.is_file():
            with binary.open("rb") as stream:
                result[binary.name] = hashlib.file_digest(stream, "sha256").hexdigest()
    # Extension executables live outside bin; record those actual shipped bytes too.
    for path in sorted((stage / "share/extensions").glob("*/extension.json")):
        program = path.parent / json.loads(path.read_text())["command"][0]
        with program.open("rb") as stream:
            result[str(program.relative_to(stage))] = hashlib.file_digest(stream, "sha256").hexdigest()
    return result


def clear_modules(stage):
    for path in (stage / "share/modules").glob("*/module.json"):
        program = Path(json.loads(path.read_text())["command"][0])
        if program.parent != Path("bin"):
            raise SystemExit(f"Unexpected bundled module command: {program}")
        (stage / program).unlink(missing_ok=True)
    for name in ("proteus-hook-worker", "proteus-claude-hooks-inspect"):
        (stage / "bin" / name).unlink(missing_ok=True)
    shutil.rmtree(stage / "share/modules", ignore_errors=True)


def publish(stage, destination):
    """Rollback on a failed final rename; the old package survives all build failures."""
    previous = destination.with_name(destination.name + ".previous")
    if previous.exists():
        raise SystemExit(f"Unresolved previous package at {previous}; inspect it before retrying")
    had_previous = destination.exists()
    if had_previous:
        destination.rename(previous)
    try:
        stage.rename(destination)
    except BaseException:
        if had_previous:
            previous.rename(destination)
        raise
    if had_previous:
        shutil.rmtree(previous)
