# Local gpui-base patch

Source: crates.io `gpui-base` 0.8.1, upstream https://github.com/longbridge/gpui-kit.
The upstream source and manifest are retained under Apache-2.0.

The shared input engine rebinds window focus, blur and activation listeners when
an existing Input/Textarea/Editor entity moves to another native window. Its
text, selections, undo manager and language state remain in the same entity.
Window deactivation stops cursor timers; activation resumes only a focused field.
The render path checks the window identity once, without subscriptions or
notifications while it is unchanged. `rehost` also permits explicit rebinding
before focusing the moved field.

Selectable HTML can register format-independent atomic inline nodes by an
attribute via `MarkdownExtensions::html_inline` and `inline_renderer`. The HTML
parser collects plain fallback only for a matching attribute and keeps custom
objects through inherited marks, lists and table cells. The existing inline
flow handles baseline, wrapping, accessibility and selection; it knows no
content languages or extension packages. Changing parser configuration also
invalidates HTML; renderer-only updates retain the parsed tree and selection.
Proteus uses this for retained content slots rendered by language providers.

Each deferred inline measurement captures its own immutable layout key. HTML
tables scope their surface, rows, cells and intrinsic probes separately, so
spanless cells cannot substitute another cell's items or one-shot element.
The existing bounded width cache remains shared only for compatible inputs;
repeated width probes of the same request reuse its key by Rc identity.

Spanless HTML paragraphs use their retained paragraph-state identity for native
element IDs. Separate paragraphs therefore retain separate text fragments,
including surrounding text before and after atomic content; Markdown source
span IDs remain unchanged.

Native inline objects compensate for nearest-device-pixel layout rounding:
a baseline that exceeds the measured height by at most half a device pixel
is clamped to that height. Larger overshoots, negative or non-finite baselines
still use the existing literal-text fallback.


## Audit corrections, 2026-11-07

Dock drag completion emits LayoutChanged. RopeLines exhausted length stays
zero. AutoGrow min/max remain valid across constructors and rows setters.
Native character lookup applies per-line scroll/gutter geometry. LSP positions
use UTF-16, edit batches resolve original coordinates and apply in reverse,
and inline completions honor replacement ranges. Fold candidate replacement
updates retained ranges and invalidates the projection. Resizable panels track
initial measurement separately from the valid 100px minimum; Both scrollbars
reserve a corner only when both axes actually overflow. Root keyboard Copy
preserves selected whitespace. Slider accessibility retains range variants and
emits semantic user changes; pointer snapping is relative to min and clamped.
Default-open popovers run focus/dismiss lifecycle. Initially absent Presence
stays absent; reduced keyframes preserve final playback direction. Area cache
includes missing endpoint inputs and plot text honors explicit font weight.
Application reachability differs per API; source-only findings are not claims
of reproduced Desktop failures. See focused audit regression tests in sources.


Rich-text corrections: decoded HTML attributes are serialized with correct
escaping; ordered HTML lists retain start. Source Markdown overlap is segmented
into disjoint combined mark runs. Inline backtick spans use inline source mapping.
Reference definitions are retained and trigger full reparsing only for documents
with that global dependency, including incremental forward/backward definitions.
Table width cache includes explicit inline-code style metrics. Virtual-list
geometry invalidates on axis/gap changes and horizontal inferred height uses the
height constraint. Keyboard tree movement skips disabled items and expansion
checks disability. HoverPopoverState exposes an epoch for same-range replacements.

Verification boundary: standalone vendor test compilation is possible through
an isolated manifest with upstream dev dependencies. The broad baseline suite
has existing failures; it is not reported as a green application release gate.
Focused regressions and default Desktop checking are reported separately.
