"""Validate existing runtime artifacts without resolving sources or building them."""
import json
import os
from pathlib import Path
import shutil


def executable(command, *, cwd=None, path=None):
    """Resolve the command exactly where the eventual process will run."""
    candidate = Path(command).expanduser()
    if candidate.is_absolute() or "/" in command or "\\" in command:
        if not candidate.is_absolute():
            candidate = Path(cwd or Path.cwd()) / candidate
        return candidate.resolve() if candidate.is_file() and os.access(candidate, os.X_OK) else None
    found = shutil.which(command, path=path)
    return Path(found).resolve() if found else None


def require_extensions(directory, *, hint, path=None, native=None):
    """Check every declared package command before starting Cargo or a UI."""
    directory = Path(directory)
    if native is not None:
        from portable_state import desktop_protocol
        protocol = desktop_protocol(native)
    else:
        protocol = None
    manifests = sorted(directory.glob("*/extension.json"))
    if not manifests:
        raise SystemExit(f"No ready extension packages in {directory}. {hint}")
    for manifest in manifests:
        try:
            command = json.loads(manifest.read_text())["command"]
            if not isinstance(command, list) or not command or not all(isinstance(arg, str) for arg in command):
                raise ValueError("command must be a nonempty string array")
            if protocol is not None and json.loads(manifest.read_text()).get("protocol") != protocol:
                raise ValueError(f"protocol differs from Desktop {protocol}; select compatible ready extensions or --build-extensions explicitly")
            if not executable(command[0], cwd=manifest.parent, path=path):
                raise ValueError(f"missing executable {command[0]}")
        except (OSError, ValueError, KeyError, TypeError) as error:
            raise SystemExit(f"Extension {manifest.parent.name}: {error}. {hint}") from error
    return directory


def require_modules(base):
    """Module command paths are relative to the portable runtime root."""
    base = Path(base)
    manifests = sorted((base / "share/modules").glob("*/module.json"))
    hint = "Provide --base PATH with ready module metadata and binaries."
    if not manifests:
        raise SystemExit(f"No ready module metadata in {base / 'share/modules'}. {hint}")
    for manifest in manifests:
        try:
            command = json.loads(manifest.read_text())["command"]
            if not isinstance(command, list) or not command or not all(isinstance(arg, str) for arg in command):
                raise ValueError("command must be a nonempty string array")
            if not executable(command[0], cwd=base):
                raise ValueError(f"missing executable {command[0]}")
        except (OSError, ValueError, KeyError, TypeError) as error:
            raise SystemExit(f"Module {manifest.parent.name}: {error}. {hint}") from error
    return base
