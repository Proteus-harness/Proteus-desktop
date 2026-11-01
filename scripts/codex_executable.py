"""Discover installed Codex once at launch, without a login shell or PATH edits."""
import os
from pathlib import Path
import shutil
import sys

LINUX_CODEX_PROGRAM = Path("/usr/lib/chatgpt/resources/codex")


def codex_program(inherited):
    """Keep an explicit override authoritative, including an unavailable path."""
    explicit = inherited.get("PROTEUS_CODEX_EXECUTABLE")
    if explicit:
        return explicit
    found = shutil.which("codex", path=inherited.get("PATH", os.defpath))
    if found:
        return str(Path(found).absolute())
    candidates = []
    prefix = inherited.get("npm_config_prefix") or inherited.get("NPM_CONFIG_PREFIX")
    if prefix:
        candidates.append(Path(prefix) / "bin/codex")
    home = inherited.get("HOME")
    if home:
        candidates.extend(Path(home) / directory / "codex"
                          for directory in (".local/bin", ".npm-global/bin", ".cargo/bin"))
    if sys.platform.startswith("linux"):
        candidates.append(LINUX_CODEX_PROGRAM)
    for candidate in candidates:
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate.absolute())
    return None


def configure_codex(environment):
    program = codex_program(environment)
    if program:
        environment["PROTEUS_CODEX_EXECUTABLE"] = program
