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
