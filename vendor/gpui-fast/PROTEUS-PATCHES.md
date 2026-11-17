# Local layout API

`GridTemplate::fractional(count, factor)` represents fractional flexible tracks
in the existing grid API. Ordinary grid helpers still use `1fr`; conversion to
Taffy and the fast-layout fingerprint include the fraction. A changed fraction
therefore invalidates layout rather than reusing a stale track size.

Desktop uses this for disclosure height: the child stays in the current layout
tree, so wrapping, nested disclosures and reversal share the same geometry.
Regressions live in Desktop's `ui::motion::reveal::layout_tests` and exercise
actual Taffy layout and hidden hitboxes. Standalone upstream tests still require
development fixtures absent from the published vendor package.
