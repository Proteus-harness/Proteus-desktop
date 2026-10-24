# Proteus downstream patch: PopupMenu surface appearance

This directory contains `gpui-component` 0.7.1 from the crates.io registry,
with a narrow public `PopupMenu::appearance(bool)` option. The original
normalized and workspace manifests are retained as `Cargo.toml` and
`Cargo.toml.orig`; the original Apache-2.0 license is `LICENSE.upstream`.
`LICENSE-APACHE` links to the repository's Apache-2.0 license.

Registry source: `gpui-component` 0.7.1, crates.io archive
`gpui-component-0.7.1.crate`, SHA-256
`bc8b7def8e74b0d822d3e96f6b28fb5a2c20eb7a83d30d22fa59ca8088d1159b`.

The patch only makes the menu's existing `popover_style` decoration optional;
it defaults to enabled, so existing menus and submenus retain their native
appearance. It does not change menu layout, item highlighting, or interactions.
