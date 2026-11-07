"""Owned Linux replay lifecycle, including independently grouped PTY children.

The standalone runner launches one fixture and becomes its child subreaper;
only its direct/adopted children are reaped. No global process-name matching.
"""
import ctypes
import os
from pathlib import Path
import signal
import subprocess
import time


def enable_subreaper():
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0):
        raise OSError(ctypes.get_errno(), "Cannot enable replay child subreaper")


def cleanup(process):
    if process.poll() is None:
        for sig in (signal.SIGTERM, signal.SIGKILL):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=3)
                break
            except subprocess.TimeoutExpired:
                continue
        if process.poll() is None:
            raise RuntimeError("Replay process did not exit")
    deadline = time.monotonic() + 4
    while True:
        while True:
            try:
                pid, _ = os.waitpid(-1, os.WNOHANG)
            except ChildProcessError:
                return
            if not pid:
                break
        children = Path(f"/proc/{os.getpid()}/task/{os.getpid()}/children").read_text().split()
        for child in children:
            try:
                os.kill(int(child), signal.SIGKILL if time.monotonic() > deadline - 2 else signal.SIGTERM)
            except ProcessLookupError:
                pass
        if time.monotonic() > deadline:
            raise RuntimeError("Owned replay children did not reap")
        time.sleep(.02)
