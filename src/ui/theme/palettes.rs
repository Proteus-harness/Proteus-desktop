//! Ready-made GPUI Kit theme families. Color values stay upstream-authored;
//! semantic field-name adaptations and source revision live in SOURCES.json.

pub(super) const BUNDLED: &[&str] = &[
    include_str!("palettes/catppuccin.json"),
    include_str!("palettes/tokyonight.json"),
    include_str!("palettes/gruvbox.json"),
    include_str!("palettes/everforest.json"),
    include_str!("palettes/solarized.json"),
    include_str!("palettes/ayu.json"),
];
