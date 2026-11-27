# GPUI Fast Linux

Sources and Apache-2.0 license from the `gpui-fast-linux` 0.1.12 crate.
Upstream repository is [longbridge/gpui-fast](https://github.com/longbridge/gpui-fast).

When synchronous input makes an active, visible Wayland window dirty, wait
one refresh interval for the pending frame callback. If the callback is late,
give that same callback one extra opportunity to submit an updated frame.
Normal callbacks still pace animations and idle windows.
