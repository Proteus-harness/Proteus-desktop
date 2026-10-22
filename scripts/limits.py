"""Keeps heavy local builds from taking the whole machine.

`limit()` re-runs the calling script in a systemd user scope with a CPU and
memory cap, at the lowest CPU and disk priority, and caps cargo jobs. Defaults:
40% of the CPUs and of the memory. `PROTEUS_BUILD_SHARE=70` changes the share,
`PROTEUS_BUILD_SHARE=100` turns the cap off; CI (`CI` set) runs uncapped.
"""
import math
import os
import shutil
import sys

MARK = "PROTEUS_BUILD_LIMITED"


def memory_mib():
    try:
        with open("/proc/meminfo") as meminfo:
            for line in meminfo:
                if line.startswith("MemTotal:"):
                    return int(line.split()[1]) // 1024
    except OSError:
        pass
    return None


def limit():
    share = int(os.environ.get("PROTEUS_BUILD_SHARE", "40"))
    if share >= 100 or os.environ.get(MARK) or os.environ.get("CI"):
        return
    os.environ[MARK] = "1"
    cpus = os.cpu_count() or 1
    os.environ.setdefault("CARGO_BUILD_JOBS", str(max(1, math.ceil(cpus * share / 100))))
    command = [sys.executable, *sys.argv]
    if sys.platform.startswith("linux") and shutil.which("systemd-run") and os.environ.get("XDG_RUNTIME_DIR"):
        properties = ["-p", f"CPUQuota={cpus * share}%"]
        memory = memory_mib()
        if memory:
            high = memory * share // 100
            properties += ["-p", f"MemoryHigh={high}M", "-p", f"MemoryMax={high + memory // 8}M"]
        command = ["systemd-run", "--user", "--scope", "--quiet", *properties, *command]
    if shutil.which("ionice"):
        command = ["ionice", "-c", "3", *command]
    if shutil.which("nice"):
        command = ["nice", "-n", "19", *command]
    try:
        os.execvp(command[0], command)
    except OSError:
        # No scope available: keep only the job cap and a low priority.
        os.nice(19)
