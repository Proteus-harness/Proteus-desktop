# Vendored ghostty-vt

Source: crates.io `ghostty-vt 0.8.0+ghostty.1.4.0-main.83fb956`,
https://github.com/prabirshrestha/gpui-cn at
e49d909b2ceae040714aa53cd38f97faa4f85cd2 (Apache-2.0 license option).
Original published manifest is retained as Cargo.toml.orig; LICENSE and
LICENSE-GHOSTTY retain upstream notices. The sys crate pins Ghostty
83fb95624b0439ee660c2679b43b38c8b63769cd and verifies the official source
archive SHA256 ed57ae9b95581c88f2614e976c1d3f146c02d443a3c26139200eb9de6e645eb4.

Local changes: Cargo rust-version lowered from 1.96 to 1.95 after compiling
the safe API and bindings with the Desktop's pinned Rust 1.95.0. No terminal
parser or renderer code changes. The Ghostty engine has dedicated native
PTY/VT tests in crates/terminal. Zig 0.17.0 is required only at build time.
