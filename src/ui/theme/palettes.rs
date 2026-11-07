//! GPUI Kit theme imports with explicit builtin surface-role adaptations.
//! SOURCES.json records the import; ROLE-MAPPING.json pins named author/port
//! tokens used for chrome, sidebar, content, cards, popup surfaces and borders.

pub(super) const BUNDLED: &[&str] = &[
    include_str!("palettes/catppuccin.json"),
    include_str!("palettes/tokyonight.json"),
    include_str!("palettes/gruvbox.json"),
    include_str!("palettes/everforest.json"),
    include_str!("palettes/solarized.json"),
    include_str!("palettes/ayu.json"),
];

#[cfg(test)]
mod tests;
