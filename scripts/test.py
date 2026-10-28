#!/usr/bin/env python3
"""Run the client workspace gate, or an explicit headless package check."""
import os
import subprocess
import sys
from sources import ROOT, resolve
from limits import limit
from check_licenses import check


HEADLESS_PACKAGES = {"proteus-diff", "proteus-tabs"}


def headless_only(args):
    """Recognize a conservative subset of Cargo arguments, never libtest args.

    Unknown options and any broader/foreign selection retain the full gate.
    This is a package check, not a replacement for workspace qualification.
    """
    flags = {"--lib", "--doc", "--no-run", "--no-fail-fast", "--locked",
             "--frozen", "--offline", "-q", "--quiet", "-v", "-vv", "--verbose"}
    valued = {"-j", "--jobs", "--color", "--message-format"}
    packages = []
    filters = 0
    index = 0
    while index < len(args):
        arg = args[index]
        if arg == "--":
            break
        option, separator, value = arg.partition("=")
        if arg in ("-p", "--package") or arg in valued:
            index += 1
            if index == len(args) or args[index].startswith("-") or not args[index]:
                return False
            if arg in ("-p", "--package"):
                packages.append(args[index])
        elif option == "--package" and separator:
            packages.append(value)
        elif arg.startswith("-p") and not arg.startswith("--"):
            packages.append(arg[2:])
        elif arg in flags:
            pass
        elif option in valued and separator and value and not value.startswith("-"):
            pass
        elif arg.startswith("-j") and arg[2:].isdigit():
            pass
        elif not arg.startswith("-") and arg and not filters:
            filters += 1
        else:
            return False
        index += 1
    return bool(packages) and all(package in HEADLESS_PACKAGES for package in packages)


def main(args):
    limit()
    env = os.environ.copy()
    headless = headless_only(args)
    if not headless:
        extensions = resolve("extensions")
        env["PROTEUS_TEST_EXTENSIONS_SOURCE"] = str(extensions)
        env["PROTEUS_EXTENSIONS_DIR"] = str(extensions / "packages")
    env.setdefault("RUST_TEST_THREADS", "4")
    problems = check(ROOT) + ([] if headless else check(extensions))
    if problems:
        raise SystemExit("\n".join(problems))
    if headless:
        print("Targeted headless package check; not the client workspace gate.", file=sys.stderr)
    else:
        # Packages are Rust binaries; --dev places them next to their manifests.
        subprocess.run([sys.executable, str(extensions / "package.py"), "--dev"], check=True, env=env)
    selection = [] if headless else ["--workspace"]
    return subprocess.call(["cargo", "test", "--locked", *selection, *args], cwd=ROOT, env=env)


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
