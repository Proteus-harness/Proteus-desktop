# Source and local changes

Vendored from the unchanged crates.io `gpui-fast` 0.1.12 package.
Package SHA256: `87b6d2f355531132e8f1ec357cb23901aba1d1b3e522260822ae199f02ec32da`. The original Apache-2.0
license is retained in LICENSE. Cargo resolves this fork through the root
`[patch.crates-io]`; the Cargo registry copy is never modified.

Local changes add explicit opt-in window-owned typed globals. Existing
unregistered application globals keep their behavior. Values are selected
using GPUI's window execution stack, never keyboard focus and never by
swapping application-global theme values. Initial/direct draw, nested window
updates, motion preference and closed-window cleanup use the same ownership.

The framework's global dependency stamps still invalidate conservatively by
type; this change makes no claim to reduce redraw work. Retained subtrees
remain owned by their original Window. Windowless model/entity callbacks must
use their known WindowHandle to update a visual owner.

Modified original files:

- `src/app.rs`: global accessor/lease routing and native window lifetime.
- `src/window.rs`: initial and direct drawing uses its exact window context.
- `src/app/async_context.rs`: the generic asynchronous window read preserves
  its owner, just like the inherent window-specific methods.
- `src/app/test_app.rs`: immutable test-window reads use the real window path.
- `src/fast/dependencies.rs`: presence insertion checks the selected map.

The source package's original lib-test target references an image and a
`util_macros` dev-dependency absent from its published archive. It is not
patched here. Local boundary regressions live in the consuming application's
`src/window_global_tests.rs`, compiled against this library with test-support.
