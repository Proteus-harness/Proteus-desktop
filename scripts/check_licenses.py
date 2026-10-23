#!/usr/bin/env python3
"""Checks per-crate licenses, as Zed's script/check-licenses does.

Each crate below the repository root declares `license` in Cargo.toml and keeps
a symlink LICENSE-GPL or LICENSE-APACHE to the root file of the same name. An
Apache-2.0 crate must not carry code adapted from GPL crates of Zed.
"""
import re
import sys
import tomllib
from pathlib import Path

LICENSES = {"GPL-3.0-or-later": "LICENSE-GPL", "Apache-2.0": "LICENSE-APACHE"}
# Zed crates published under Apache-2.0 (`license` in their Cargo.toml).
ZED_APACHE = re.compile(r"crates/(gpui\w*|collections|refineable|sum_tree|util|util_macros|scheduler"
                        r"|http_client|path|watch|zlog|ztracing\w*|extension_api)/")
ADAPTED = re.compile(r"Адаптировано из Zed:(.*)")


def license_of(manifest, workspace):
    package = tomllib.loads(manifest.read_text()).get("package", {})
    value = package.get("license")
    if isinstance(value, dict) and value.get("workspace"):
        value = workspace.get("license")
    if "license-file" in package:
        return None, f"{manifest}: use `license`, not `license-file`"
    return value, None


def check(root):
    root = Path(root).resolve()
    errors = []
    workspace = tomllib.loads((root / "Cargo.toml").read_text()).get("workspace", {}).get("package", {})
    for name in LICENSES.values():
        if not (root / name).is_file() or (root / name).is_symlink():
            errors.append(f"{root / name}: the root keeps the license text itself")
    for manifest in sorted(root.glob("**/Cargo.toml")):
        if any(part in ("target", "build") for part in manifest.relative_to(root).parts):
            continue
        crate = manifest.parent
        if "package" not in tomllib.loads(manifest.read_text()):
            continue
        value, error = license_of(manifest, workspace)
        if error:
            errors.append(error)
            continue
        if value not in LICENSES:
            errors.append(f"{manifest}: license must be one of {', '.join(LICENSES)}, not {value!r}")
            continue
        if crate != root:
            link = crate / LICENSES[value]
            expected = Path(*[".."] * len(crate.relative_to(root).parts), LICENSES[value])
            if not link.is_symlink() or Path(link.readlink()) != expected:
                errors.append(f"{link}: must be a symlink to {expected}")
            for other in LICENSES.values():
                if other != LICENSES[value] and (crate / other).exists():
                    errors.append(f"{crate / other}: contradicts license {value}")
        if value == "Apache-2.0":
            for source in sorted(crate.glob("**/*.rs")):
                if "target" in source.relative_to(crate).parts:
                    continue
                for origin in ADAPTED.findall(source.read_text()):
                    sources = [part.strip() for part in origin.split(",")]
                    if not all(ZED_APACHE.match(part) for part in sources):
                        errors.append(f"{source}: GPL code of Zed ({origin.strip()}) in an Apache-2.0 crate")
    return errors


if __name__ == "__main__":
    problems = check(sys.argv[1] if len(sys.argv) > 1 else Path(__file__).resolve().parents[1])
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        raise SystemExit(1)
    print("licenses ok")
