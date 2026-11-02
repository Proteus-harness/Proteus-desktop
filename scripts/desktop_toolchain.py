"""Select build-only terminal tools for the effective Desktop Cargo features."""
import os
from pathlib import Path
import re
import tomllib

import terminal_toolchain


def needs_ghostty(root, flags=()):
    """Honor Cargo's feature switches without starting Cargo or downloading tools.

    Only Cargo arguments before ``--`` participate. Following local feature
    aliases keeps the wrapper in sync with the manifest's default engine.
    """
    flags = list(flags)
    flags = flags[:flags.index("--")] if "--" in flags else flags
    manifest = tomllib.loads((Path(root) / "Cargo.toml").read_text())
    features = manifest.get("features", {})
    package = manifest.get("package", {}).get("name", "proteus-native")
    selected = [] if "--no-default-features" in flags else ["default"]
    if "--all-features" in flags:
        selected.extend(features)
    for index, flag in enumerate(flags):
        if flag in ("--features", "-F") and index + 1 < len(flags):
            selected.extend(re.split(r"[\s,]+", flags[index + 1]))
        elif flag.startswith("--features="):
            selected.extend(re.split(r"[\s,]+", flag.partition("=")[2]))
        elif flag.startswith("-F") and len(flag) > 2:
            selected.extend(re.split(r"[\s,]+", flag[2:]))
    seen = set()
    while selected:
        feature = selected.pop()
        if feature in seen:
            continue
        seen.add(feature)
        if feature in ("ghostty-vt", "proteus-terminal/ghostty-vt"):
            return True
        if feature.startswith(package + "/"):
            feature = feature[len(package) + 1:]
        selected.extend(features.get(feature, ()))
    return False


def prepare_desktop(root, flags=(), environ=None):
    """Return an independent Cargo environment; prepare Zig only when needed."""
    environment = dict(os.environ if environ is None else environ)
    if needs_ghostty(root, flags):
        return terminal_toolchain.prepare(Path(root), environment)
    return environment
