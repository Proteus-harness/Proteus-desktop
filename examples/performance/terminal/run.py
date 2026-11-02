#!/usr/bin/env python3
"""Run the same release PTY fixture with one VT engine at a time.

Build the fixture before this command; no compilation occurs during samples.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import statistics
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]


def read_kib(text, key):
    for line in text.splitlines():
        if line.startswith(key):
            return int(line.split()[1])
    return 0


def tree_memory(pid):
    pending, seen = [pid], set()
    rss = pss = swap = 0
    while pending:
        child = pending.pop()
        if child in seen:
            continue
        seen.add(child)
        try:
            status = Path(f"/proc/{child}/status").read_text()
            smaps = Path(f"/proc/{child}/smaps_rollup").read_text()
            rss += read_kib(status, "VmRSS:")
            pss += read_kib(smaps, "Pss:")
            swap += read_kib(smaps, "Swap:")
            for task in Path(f"/proc/{child}/task").iterdir():
                pending.extend(int(value) for value in (task / "children").read_text().split())
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            pass
    return rss, pss, swap


def run(binary, engine, lines):
    environment = dict(os.environ, PROTEUS_TERMINAL_ENGINE=engine, SHELL="/bin/sh")
    cpu_before = resource.getrusage(resource.RUSAGE_CHILDREN)
    start = time.perf_counter()
    process = subprocess.Popen([str(binary), str(lines)], cwd=ROOT, env=environment,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    peak = [0, 0, 0]
    while process.poll() is None:
        sample = tree_memory(process.pid)
        peak = [max(old, current) for old, current in zip(peak, sample)]
        if time.perf_counter() - start > 100:
            process.kill()
            raise RuntimeError(f"{engine}/{lines}: fixture timeout")
        time.sleep(0.01)
    output, errors = process.communicate()
    if process.returncode:
        raise RuntimeError(f"{engine}/{lines}: {errors.strip()}")
    cpu_after = resource.getrusage(resource.RUSAGE_CHILDREN)
    result = json.loads(output)
    result.update({"wall_seconds": time.perf_counter() - start,
        "user_seconds": cpu_after.ru_utime - cpu_before.ru_utime,
        "system_seconds": cpu_after.ru_stime - cpu_before.ru_stime,
        "sampled_tree_peak_rss_kib": peak[0], "sampled_tree_peak_pss_kib": peak[1],
        "sampled_tree_peak_swap_kib": peak[2]})
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/examples/replay")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runs", type=int, default=3)
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        raise SystemExit(f"Build the terminal release example first: {binary}")
    results = []
    # Alternate engines within each workload so warm machine state is shared.
    for lines in (10_000, 100_000):
        for repetition in range(args.runs):
            for engine in ("alacritty", "ghostty"):
                result = run(binary, engine, lines)
                result["repetition"] = repetition + 1
                results.append(result)
                print(json.dumps(result), flush=True)
    medians = []
    for lines in (10_000, 100_000):
        for engine in ("alacritty", "ghostty"):
            selected = [item for item in results if item["engine"] == engine and item["lines"] == lines]
            metrics = {key: statistics.median(item[key] for item in selected)
                       for key in selected[0] if key not in ("engine", "lines", "repetition")}
            medians.append({"engine": engine, "lines": lines, **metrics})
    source = hashlib.sha256()
    for directory in (ROOT / "crates/terminal", ROOT / "vendor/ghostty-vt", ROOT / "vendor/ghostty-vt-sys"):
        for path in sorted(directory.rglob("*")):
            if path.is_file():
                source.update(str(path.relative_to(ROOT)).encode())
                source.update(path.read_bytes())
    report = {"binary": str(binary), "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "terminal_sources_sha256": source.hexdigest(),
        "ghostty_commit": "83fb95624b0439ee660c2679b43b38c8b63769cd", "rust_profile": "release",
        "grid": {"columns": 100, "rows": 30}, "shell": "/bin/sh", "sample_interval_ms": 10,
        "idle_interval_ms": 1000, "notes": "Self RSS/PSS at completion and after 1s idle; sampled tree peaks include shell/awk. Headless fixture excludes GPUI and model backends.",
        "runs": results, "medians": medians}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
