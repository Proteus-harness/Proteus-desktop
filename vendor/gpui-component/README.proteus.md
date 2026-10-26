# Proteus downstream patch: PopupMenu surface appearance

This directory contains `gpui-component` 0.8.1 from the crates.io registry,
with a narrow public `PopupMenu::appearance(bool)` option. The original
normalized and workspace manifests are retained as `Cargo.toml` and
`Cargo.toml.orig`; the original Apache-2.0 license is `LICENSE.upstream`.
`LICENSE-APACHE` links to the repository's Apache-2.0 license.

Registry source: `gpui-component` 0.8.1, crates.io archive
`gpui-component-0.8.1.crate`, SHA-256
`46ce53430332e30f303364eaabfa10ed2aa23dfaf2a147e04370e0e9c423578b`.

The patch only makes the menu's existing `popover_style` decoration optional;
it defaults to enabled, so existing menus and submenus retain their native
appearance. It does not change menu layout, item highlighting, or interactions.
The GPUI Fast backend is selected through the upstream `gpui-fast` feature.
