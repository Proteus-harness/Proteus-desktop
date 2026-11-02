#!/usr/bin/env python3
"""Compare finished SVG fixture runs: provenance, painted bounds and PSS.

Pillow inspects original PNGs without editing them. Matching colored regions
does not establish full SVG fidelity or GPU allocation release.
"""
import argparse
import hashlib
import json
from pathlib import Path

from PIL import Image, ImageChops


def read(path):
    return json.loads(path.read_text())


def regions(path, colors):
    with Image.open(path) as source:
        frame = source.convert("RGB")
    channels = frame.split()
    result = []
    for color in colors:
        masks = [channel.point(lambda pixel, expected=value: 255 if pixel == expected else 0)
                 for channel, value in zip(channels, color)]
        mask = ImageChops.multiply(ImageChops.multiply(masks[0], masks[1]), masks[2])
        result.append({"bounds": mask.getbbox(), "pixels": mask.histogram()[255]})
    return {"size": frame.size, "regions": result}


def load_case(path):
    report = read(path / "report.json")
    pixels = read(path / "pixel-verification.json")
    if report["status"] != "stopped" or report["fixture_errors"] or pixels["status"] != "passed":
        raise ValueError(f"Run did not finish cleanly: {path}")
    captures = {}
    for capture in pixels["captures"]:
        name = "cleared" if capture["empty"] else f"stage-{capture['stage']:02}"
        if name in captures:
            raise ValueError(f"Duplicate capture: {name}")
        png = path / capture["screenshot"]
        if hashlib.sha256(png.read_bytes()).hexdigest() != capture["sha256"]:
            raise ValueError(f"Screenshot hash changed: {png}")
        captures[name] = regions(png, capture["expected_colors"])
    expected = {f"stage-{index:02}" for index in range(16)} | {"cleared"}
    if set(captures) != expected or set(report["marked_samples"]) != expected:
        raise ValueError(f"Incomplete captures or acknowledged samples: {path}")
    return report, pixels, captures


def normalized_manifest(path, report):
    manifest = path / "extensions/memory-svg/extension.json"
    if hashlib.sha256(manifest.read_bytes()).hexdigest() != report["extension_manifest_sha256"]:
        raise ValueError(f"Installed manifest hash changed: {manifest}")
    value = read(manifest)
    command = value["command"]
    if len(command) != 3 or command[2] != str((path / "rendered.jsonl").resolve()):
        raise ValueError("Unexpected renderer receipt destination")
    # Each isolated run necessarily writes receipts into its own output.
    command[2] = "<run>/rendered.jsonl"
    return value


def compare(before, after):
    old, old_pixels, old_captures = load_case(before)
    new, new_pixels, new_captures = load_case(after)
    errors = []
    for key in ("source_sha256", "corpus_sha256",
                "transport_sha256", "launch_helpers_sha256", "sampler_sha256"):
        if old[key] != new[key]:
            errors.append(f"Input provenance changed: {key}")
    if normalized_manifest(before, old) != normalized_manifest(after, new):
        errors.append("Installed renderer manifest changed beyond its receipt path")
    for key in ("helper_sha256", "settle_seconds", "initial_stage"):
        if old_pixels[key] != new_pixels[key]:
            errors.append(f"Capture procedure changed: {key}")
    for name in old_captures:
        a, b = old_captures[name], new_captures[name]
        if a["size"] != b["size"]:
            errors.append(f"Viewport changed: {name}")
        if [r["bounds"] for r in a["regions"]] != [r["bounds"] for r in b["regions"]]:
            errors.append(f"Painted bounds changed: {name}")
    memory = {}
    for name in ("stage-00", "stage-07", "stage-15", "cleared"):
        memory[name] = {}
        for label, report in (("before", old), ("after", new)):
            sample = report["marked_samples"][name]
            native = sample["groups"]["gui"]
            memory[name][label] = {
                "native_pss_mib": round(native["pss_kib"] / 1024, 3),
                "native_rss_mib": round(native["rss_kib"] / 1024, 3),
                "tree_pss_mib": round(sample["pss_kib"] / 1024, 3),
                "tree_rss_mib": round(sample["rss_kib"] / 1024, 3),
                "tree_processes": sample["process_count"],
            }
    return {"passed": not errors, "errors": errors, "memory": memory,
            "before_elf": old["executable_sha256"], "after_elf": new["executable_sha256"],
            "corpus_sha256": old["corpus_sha256"],
            "before_bounds": old_captures, "after_bounds": new_captures,
            "limits": "Exact bounds of two painted color regions per stage, not full SVG fidelity, GPU VRAM, long-lived leak proof or a PSS cap. Memory has no pass/fail threshold."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--before", type=Path, required=True)
    parser.add_argument("--after", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.before, args.after)
    result["comparison_helper_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    with args.output.open("x") as stream:
        json.dump(result, stream, ensure_ascii=False, indent=2)
        stream.write("\n")
    print(json.dumps({key: result[key] for key in ("passed", "errors", "memory")}, ensure_ascii=False, indent=2))
    raise SystemExit(0 if result["passed"] else 1)


if __name__ == "__main__":
    main()
