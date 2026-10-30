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
