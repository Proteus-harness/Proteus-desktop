#!/usr/bin/env python3
"""Build/run Desktop, using ready runtime components from an existing portable."""
import argparse
import os
from pathlib import Path
import subprocess
import sys

from sources import ROOT, resolve
from runtime_paths import executable, require_extensions, require_modules


def options(args):
    # Only runner flags before `--` are consumed. All application arguments stay
    # in their original order, including flags unknown to this wrapper.
    before, separator, after = list(args), False, []
    if "--" in before:
        index = before.index("--")
        before, after, separator = before[:index], before[index + 1:], True
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", type=Path, default=ROOT / "build/Proteus",
                        help="existing portable runtime (default: build/Proteus)")
    parser.add_argument("--build-extensions", action="store_true",
                        help="explicitly rebuild source extension packages")
    config, application = parser.parse_known_args(before)
    return config, application + after if separator else application


def main(args):
    config, application = options(args)
    env = os.environ.copy()
    base = config.base.expanduser().resolve()
    env["PATH"] = str(base / "bin") + os.pathsep + env.get("PATH", "")
    hint = "Provide an existing portable with --base PATH or set PROTEUS_BIN to a ready backend."
    # An attached client does not launch a local backend or its modules.
    if "--origin" not in application:
        backend = executable(env.get("PROTEUS_BIN", str(base / "bin/proteus")), path=env["PATH"])
        if backend is None:
            raise SystemExit(f"Ready Proteus backend not found. {hint}")
        env["PROTEUS_BIN"] = str(backend)
        require_modules(base)
    if config.build_extensions:
        extensions = resolve("extensions")
        subprocess.run([sys.executable, str(extensions / "package.py"), "--dev"], check=True, env=env)
        directory = extensions / "packages"
    else:
        directory = Path(env.get("PROTEUS_EXTENSIONS_DIR", str(base / "share/extensions"))).expanduser().resolve()
    require_extensions(directory, hint="Use --base PATH with ready extensions, set PROTEUS_EXTENSIONS_DIR, or explicitly pass --build-extensions.", path=env["PATH"], native=ROOT)
    env["PROTEUS_EXTENSIONS_DIR"] = str(directory)
    print("Building/running Desktop only; using ready backend, modules and extensions.", file=sys.stderr)
    return subprocess.call(["cargo", "run", "--locked", "-p", "proteus-native", "--", *application], cwd=ROOT, env=env)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
