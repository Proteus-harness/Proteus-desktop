# Vendored ghostty-vt

Source: crates.io `ghostty-vt 0.8.0+ghostty.1.4.0-main.83fb956`,
https://github.com/prabirshrestha/gpui-cn at
e49d909b2ceae040714aa53cd38f97faa4f85cd2 (Apache-2.0 license option).
Original published manifest is retained as Cargo.toml.orig; LICENSE and
LICENSE-GHOSTTY retain upstream notices. The sys crate pins Ghostty
83fb95624b0439ee660c2679b43b38c8b63769cd and verifies the official source
archive SHA256 ed57ae9b95581c88f2614e976c1d3f146c02d443a3c26139200eb9de6e645eb4.

Local changes: Cargo rust-version lowered from 1.96 to 1.95 after compiling
the safe API and bindings with the Desktop's pinned Rust 1.95.0. The native Ghostty engine remains pinned; Rust wrapper corrections are recorded below. The Ghostty engine has dedicated native
PTY/VT tests in crates/terminal. Zig 0.17.0 is required only at build time.


Audit corrections (2026-11-07): grapheme destination capacity checked before
FFI; newly allocated safe Bytes initialized to zero; raw SGR slices converted
only internally under parser borrow, including zero-length/null handling.
Callback-allocator allocation is explicitly unsafe: callers must release or
transfer ownership before callback state expires. Search snapshot APIs verify
terminal identity; placement iteration holds the graphics borrow. OSC accepts
invalid/null commands and decodes the native const-char pointer with CStr.
Resource license documentation preserves Kitty-derived GPL notices. These are
wrapper API corrections; they do not establish a previously exploitable Desktop
path (Desktop uses length-carrying UTF-8 rendering and disables Kitty graphics).
