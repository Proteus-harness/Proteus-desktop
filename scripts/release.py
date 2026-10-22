#!/usr/bin/env python3
"""Pinned-source Linux release preparation; publication belongs to the tag workflow."""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
from pathlib import Path

from release_metadata import VERSION_PATTERN, manifest, version

ROOT = Path(__file__).resolve().parent.parent
BUILD = ROOT / "build"
SPECS = json.loads((ROOT / "repositories.json").read_text())


def run(args, cwd=ROOT, **kwargs):
    print("+", " ".join(map(str, args)), flush=True)
    subprocess.run(list(map(str, args)), cwd=cwd, check=True, **kwargs)


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root, text=True).strip()


def paths():
    return {name: Path(os.environ.get(spec["env"], BUILD / "sources" / name)).resolve()
            for name, spec in SPECS.items()}


def prepare():
    """Ignore siblings and ambient source overrides; CI always builds committed pins."""
    lines = []
    for name, spec in SPECS.items():
        if not re.fullmatch(r"[0-9a-f]{40}", spec["rev"]):
            raise SystemExit(f"{name} must pin a full commit SHA")
        destination = BUILD / "sources" / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        if not destination.exists():
            run(["git", "clone", "--filter=blob:none", "--no-checkout", spec["url"], destination])
        if git(destination, "remote", "get-url", "origin") != spec["url"]:
            raise SystemExit(f"Unexpected source origin: {name}")
        run(["git", "fetch", "--depth=1", "origin", spec["rev"]], destination)
        run(["git", "checkout", "--detach", spec["rev"]], destination)
        lines.append(f"{spec['env']}={destination}")
    if os.environ.get("GITHUB_ENV"):
        with open(os.environ["GITHUB_ENV"], "a") as output:
            output.write("\n".join(lines) + "\n")
    else:
        print("Set these exact source paths for subsequent commands:")
        print("\n".join(lines))


def package_version(path):
    return tomllib.loads(path.read_text())["package"]["version"]


def check_sources(tag=None):
    source = paths()
    for name, path in {"desktop": ROOT, **source}.items():
        if git(path, "status", "--porcelain", "--untracked-files=normal"):
            raise SystemExit(f"Release source checkout is dirty: {name}")
        if name != "desktop":
            spec = SPECS[name]
            if git(path, "rev-parse", "HEAD") != spec["rev"]:
                raise SystemExit(f"Release source does not match repositories.json: {name}")
            if not (path / spec["marker"]).is_file():
                raise SystemExit(f"Missing source marker: {name}")
    release_version = version(ROOT)
    expected = {
        "Core": source["harness"] / "crates/proteus-core/Cargo.toml",
        "Modules": source["modules"] / "components/process-module/Cargo.toml",
    }
    for name, path in expected.items():
        if package_version(path) != release_version:
            raise SystemExit(f"{name} package version must equal Desktop {release_version}")
    if tag and (not re.fullmatch("v" + VERSION_PATTERN, tag) or tag != "v" + release_version):
        raise SystemExit(f"Release tag must be v{release_version}; received {tag!r}")
    if os.environ.get("GITHUB_REF_TYPE") == "tag" and not tag:
        raise SystemExit("A tag build must explicitly validate its release tag")
    return source


def sdk_gate(core):
    # Keep the starter an independent Cargo workspace with its own Git pin/lock,
    # while reusing this runner's compiled dependencies rather than another target.
    env = os.environ.copy()
    target = Path(env.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = core / target
    target = target.resolve()
    env["CARGO_TARGET_DIR"] = str(target)
    run(["cargo", "build", "--locked", "--manifest-path", "examples/modules/rust-tool/Cargo.toml"], core, env=env)
    run(["cargo", "build", "--locked", "-p", "proteus-module-protocol", "--bin", "proteus-component-conformance"], core, env=env)
    run([sys.executable, "examples/modules/rust-tool/smoke.py",
         "--conformance", target / "debug/proteus-component-conformance",
         "--module", target / "debug/proteus-rust-tool-example"], core, env=env)


def gates(section="all"):
    source = check_sources(os.environ.get("PROTEUS_RELEASE_TAG"))
    for name, path in (("core", source["harness"]), ("modules", source["modules"]), ("desktop", ROOT)):
        if section in ("all", name):
            run(["cargo", "fmt", "--all", "--", "--check"], path)
            run([sys.executable, "scripts/test.py", *([] if name == "desktop" else ["full"])], path)
            if name == "core":
                sdk_gate(path)
    if section in ("all", "extensions"):
        run(["cargo", "fmt", "--all", "--", "--check"], source["extensions"])
        run(["cargo", "test", "--locked", "--workspace"], source["extensions"])
        run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-p", "test_*.py"], source["extensions"])
    if section in ("all", "desktop"):
        run([sys.executable, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_release*.py"])
    check_sources(os.environ.get("PROTEUS_RELEASE_TAG"))


def source_archive(source, destination):
    """Export committed sources, never local credentials, target or real sessions."""
    with tempfile.TemporaryDirectory(prefix="proteus-release-sources-") as temporary:
        exported = Path(temporary) / "sources"
        exported.mkdir()
        for name, path in {"Proteus-desktop": ROOT, **{
            {"harness": "Proteus-core", "modules": "Proteus-modules",
             "extensions": "Proteus-desktop-extensions"}[key]: value
            for key, value in source.items()
        }}.items():
            snapshot = Path(temporary) / (name + ".tar")
            run(["git", "archive", "--format=tar", f"--prefix={name}/", "-o", snapshot, "HEAD"], path)
            with tarfile.open(snapshot) as archive:
                archive.extractall(exported, filter="data")
        with tarfile.open(destination, "w:gz") as archive:
            archive.add(exported, arcname="sources")


def assets():
    source = check_sources(os.environ.get("PROTEUS_RELEASE_TAG"))
    portable = BUILD / "Proteus"
    expected = manifest(ROOT, source["harness"], source["modules"], source["extensions"])
    embedded = json.loads((portable / "share/release-sources.json").read_text())
    if embedded != expected or any(record["dirty"] for record in embedded["sources"].values()):
        raise SystemExit("Portable provenance does not match the clean pinned release sources")
    machine = os.uname().machine
    name = f"Proteus-{version(ROOT)}-linux-{machine}"
    distribution = BUILD / "release"
    shutil.rmtree(distribution, ignore_errors=True)
    distribution.mkdir()
    binary_archive = BUILD / f"{name}.tar.gz"
    if not binary_archive.is_file():
        raise SystemExit("Build the release portable with scripts/portable.py first")
    shutil.copy2(binary_archive, distribution / binary_archive.name)
    sources = distribution / f"Proteus-{version(ROOT)}-sources.tar.gz"
    source_archive(source, sources)
    (distribution / "release-sources.json").write_text(json.dumps(embedded, indent=2) + "\n")
    shutil.copy2(portable / "share/licenses/README.txt", distribution / "LICENSES.txt")
    smoke = BUILD / "verification/release-smoke.json"
    if not smoke.is_file():
        raise SystemExit("Run scripts/smoke_portable.py before packaging release assets")
    smoke_record = json.loads(smoke.read_text())
    if smoke_record.get("source_manifest") != embedded or smoke_record.get("status") != "passed":
        raise SystemExit("Portable smoke evidence does not match this release")
    with binary_archive.open("rb") as stream:
        archive_digest = hashlib.file_digest(stream, "sha256").hexdigest()
    if smoke_record.get("archive_sha256") != archive_digest:
        raise SystemExit("Release archive differs from the successfully relocated archive")
    shutil.copy2(smoke, distribution / "release-smoke.json")
    checksums = []
    for path in sorted(distribution.iterdir()):
        with path.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        checksums.append(f"{digest}  {path.name}")
    (distribution / "SHA256SUMS").write_text("\n".join(checksums) + "\n")
    print(f"Release assets: {distribution}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("prepare", "check", "gates", "smoke", "assets"))
    parser.add_argument("--tag", help="must match the Desktop manifest version exactly")
    parser.add_argument("--section", choices=("all", "core", "modules", "extensions", "desktop"), default="all")
    args = parser.parse_args()
    if args.command == "prepare":
        prepare()
    elif args.command == "check":
        check_sources(args.tag)
    elif args.command == "gates":
        gates(args.section)
    elif args.command == "smoke":
        source = check_sources(os.environ.get("PROTEUS_RELEASE_TAG"))
        archive = BUILD / f"Proteus-{version(ROOT)}-linux-{os.uname().machine}.tar.gz"
        run([sys.executable, ROOT / "scripts/smoke_portable.py", archive,
             "--extensions-source", source["extensions"]])
    elif args.command == "assets":
        assets()


if __name__ == "__main__":
    main()
