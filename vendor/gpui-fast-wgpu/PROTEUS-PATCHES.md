# Local WGPU audit corrections (2026-11-07)

Solid/wavy underline alpha applies coverage once, avoiding alpha-squared
transparency. Unit-test fonts are vendored under test-assets with per-file
licenses and source hashes (SOURCES.json); these are test inputs, not a new
application font preference. Rounded clipping fixtures are copied
from the exact Cargo dependency gpui-fast 0.1.12 src/fast/rounded_clip_fixtures.rs
(Apache-2.0), with a one-bit UNORM8 tolerance for half-intensity green; clipping and alpha checks remain exact. Include paths are local and gpui TestAppContext helpers are
consistently gated behind test-support. GPU visual/latency claims require an
actual executed GPU test, distinct from shader validation or compilation.

## Emoji presentation (2026-11-11)

A supported Noto Color Emoji bitmap face is now selected for complete Unicode
emoji-presentation graphemes, even when the primary UI font has monochrome
coverage. Default text symbols remain text unless VS16 requests emoji; VS15
retains text presentation, and ordinary digits are not keycaps. Joined emoji,
modifiers and flags remain within their grapheme shaping spans. Missing faces
or unsupported bases retain the ordinary fallback path. The face lookup is
cached and invalidated by font registration; no system font scan occurs per row.

The direct unicode-properties dependency reuses the existing locked 0.1.3
Unicode emoji tables. A synthetic cmap on the existing licensed outline fixture
regresses primary-font competition, selector semantics, and actual RGB bitmap
output without requiring host fonts or new binary fixtures. Public GPUI ABI,
text contents, rasterization, and atlas contracts are unchanged.
