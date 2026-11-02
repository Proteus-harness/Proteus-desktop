"""Small deterministic sources; raster size, not source length, drives SVG churn."""
import hashlib

LANGUAGE = "memory-svg"
WIDTH, HEIGHT = 1280, 280
STAGES = 16


def source(stage):
    if type(stage) is not int or not 0 <= stage < STAGES:
        raise ValueError("stage must be an integer in 0..15")
    return f"stage={stage:02d}"


def stage_from_source(value):
    for stage in range(STAGES):
        if value == source(stage):
            return stage
    raise ValueError("source is not an exact corpus stage")


def markdown(stage):
    return f"```{LANGUAGE}\n{source(stage)}\n```"


def svgs(stage):
    source(stage)
    results = []
    for index in range(2):
        color = f"#{(stage * 63 + index * 97 + 40) % 256:02x}{(stage * 31 + index * 71 + 70) % 256:02x}{(stage * 17 + index * 43 + 100) % 256:02x}"
        results.append(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {WIDTH} {HEIGHT}">'
                       f'<rect width="{WIDTH}" height="{HEIGHT}" fill="{color}"/>'
                       '<path d="M 0 0 L 1280 280 M 0 280 L 1280 0" stroke="#ffffff" stroke-width="8"/>'
                       f'<text x="48" y="155" fill="#ffffff" font-size="56">SVG STAGE {stage:02d} / {index + 1}</text>'
                       '</svg>')
    return results


def hashes(stage):
    return [hashlib.sha256(svg.encode()).hexdigest() for svg in svgs(stage)]


def manifest():
    return {"language": LANGUAGE, "width": WIDTH, "height": HEIGHT,
            "stages": [{"stage": stage, "source": source(stage), "markdown": markdown(stage), "svg_sha256": hashes(stage)}
                       for stage in range(STAGES)]}
