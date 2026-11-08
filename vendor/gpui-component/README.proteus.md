# Proteus downstream component patches

This directory contains `gpui-component` 0.8.1 from the crates.io registry,
with downstream appearance and correctness patches. See `PROTEUS-PATCHES.md`
for the current audit dispositions and verification limits. The original
normalized and workspace manifests are retained as `Cargo.toml` and
`Cargo.toml.orig`; the original Apache-2.0 license is `LICENSE.upstream`.
`LICENSE-APACHE` links to the repository's Apache-2.0 license.

Registry source: `gpui-component` 0.8.1, crates.io archive
`gpui-component-0.8.1.crate`, SHA-256
`46ce53430332e30f303364eaabfa10ed2aa23dfaf2a147e04370e0e9c423578b`.

The original appearance patch makes the menu's existing `popover_style` decoration optional;
it defaults to enabled, so existing menus and submenus retain their native
appearance. It does not change menu layout or interactions.
The later state-role patch uses `list.hover.background` for both the pointer
highlight and keyboard cursor, matching the client's picker rows. It preserves
the renderable token (including gradients) and keeps a committed option's
selection separate from that cursor.
The GPUI Fast backend is selected through the upstream `gpui-fast` feature.
