# Local WGPU audit corrections (2026-11-07)

Solid/wavy underline alpha applies coverage once, avoiding alpha-squared
transparency. Unit-test fonts are vendored under test-assets with per-file
licenses and source hashes (SOURCES.json); these are test inputs, not a new
application font preference. Rounded clipping fixtures are copied
from the exact Cargo dependency gpui-fast 0.1.12 src/fast/rounded_clip_fixtures.rs
(Apache-2.0), with a one-bit UNORM8 tolerance for half-intensity green; clipping and alpha checks remain exact. Include paths are local and gpui TestAppContext helpers are
consistently gated behind test-support. GPU visual/latency claims require an
actual executed GPU test, distinct from shader validation or compilation.
