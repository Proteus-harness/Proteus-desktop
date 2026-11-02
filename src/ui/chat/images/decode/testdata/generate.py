"""Regenerate synthetic lossless WebP test fixtures with Pillow/libwebp.
No production input, downloads, fonts or application state are used.
Run manually: python3 src/ui/chat/images/decode/testdata/generate.py
Pillow is needed only for regeneration, not Rust builds or tests.
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).parent
for name, count, size in [("animated-40.webp", 40, 400), ("overlong-257.webp", 257, 2)]:
    frames = [Image.new("RGBA", (size, size), (255, 0, 0, 255) if index % 2 == 0 else (0, 0, 255, 255)) for index in range(count)]
    frames[0].save(root / name, format="WEBP", save_all=True, append_images=frames[1:], duration=[(index + 1) * 10 for index in range(count)], loop=0, lossless=True, minimize_size=True)
    image = Image.open(root / name)
    assert image.n_frames == count
