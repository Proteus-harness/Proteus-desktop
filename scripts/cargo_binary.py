"""Resolve one binary from Cargo's emitted artifacts, never a guessed target path."""

import json
import os
from pathlib import Path
import subprocess
import sys


def existing_binary(cwd, profile, binary):
    """Explicit reuse mode: existing conventional output, without invoking Cargo.

    This mode does not establish freshness. Normal builds must use build_binary
    to honor arbitrary Cargo configuration and select the actual build output.
    """
    cwd = Path(cwd).resolve()
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    if not target.is_absolute():
        target = cwd / target
    output = target / profile / binary
    if not output.is_file():
        raise SystemExit(f"No existing binary to reuse: {output}; run a normal build first")
    return output


def build_binary(cwd, package, binary, flags, manifest):
    """Build the selected binary and return the exact existing Cargo output.

    Cargo owns target-directory/profile/config resolution. The manifest binds the
    artifact to the requested package without parsing Cargo's opaque package_id.
    Keep the inherited environment, including Cargo target/cache configuration.
    """
    cwd = Path(cwd).resolve()
    manifest = Path(manifest).resolve()
    command = [
        "cargo", "build", "--locked", *flags, "-p", package, "--bin", binary,
        "--message-format=json-render-diagnostics",
    ]
    print("+", " ".join(command), flush=True)
    # stderr stays attached for Cargo progress/errors. Buffer only its JSON
    # stdout so failed builds can still have all rendered diagnostics forwarded.
    result = subprocess.run(
        command, cwd=cwd, stdout=subprocess.PIPE, text=True, check=False,
    )
    outputs = []
    finished = 0
    invalid = False
    for line in result.stdout.splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            print(line, file=sys.stderr)
            invalid = True
            continue
        if not isinstance(message, dict):
            invalid = True
            continue
        if message.get("reason") == "build-finished":
            finished += 1
            if message.get("success") is not True:
                invalid = True
        if message.get("reason") == "compiler-message":
            details = message.get("message")
            if not isinstance(details, dict):
                invalid = True
                continue
            diagnostic = details.get("rendered")
            if diagnostic is not None and not isinstance(diagnostic, str):
                invalid = True
                continue
            if diagnostic:
                print(diagnostic, end="" if diagnostic.endswith("\n") else "\n", file=sys.stderr)
        if message.get("reason") != "compiler-artifact":
            continue
        target = message.get("target")
        profile = message.get("profile")
        if not isinstance(target, dict) or not isinstance(profile, dict):
            invalid = True
            continue
        artifact_manifest = message.get("manifest_path")
        if not isinstance(artifact_manifest, str):
            continue
        artifact_manifest = Path(artifact_manifest)
        if not artifact_manifest.is_absolute():
            artifact_manifest = cwd / artifact_manifest
        if (
            artifact_manifest.resolve() != manifest
            or target.get("name") != binary
            or target.get("kind") != ["bin"]
            or profile.get("test", False)
        ):
            continue
        executable = message.get("executable")
        if not isinstance(executable, str) or not executable:
            invalid = True
            continue
        output = Path(executable)
        outputs.append(output if output.is_absolute() else cwd / output)
    if result.returncode:
        raise subprocess.CalledProcessError(result.returncode, command, output=result.stdout)
    if invalid or finished != 1 or len(outputs) != 1:
        raise SystemExit(
            f"Cargo did not emit exactly one valid executable and successful completion "
            f"for {package}/{binary}"
        )
    output = outputs[0]
    if not output.is_file():
        raise SystemExit(f"Cargo executable does not exist: {output}")
    return output
