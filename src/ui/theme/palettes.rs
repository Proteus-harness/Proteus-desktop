//! Builtin color themes, including adapted themes imported from GPUI Kit.
//! SOURCES.json records the import; ROLE-MAPPING.json pins named author/port
//! tokens used for chrome, sidebar, content, cards, popup surfaces, disclosure
//! and composer dock fills, plus readable labels and explicit neutral control
//! states. Proteus interpolations are recorded alongside authored source roles.

const CLAUDE: &str = include_str!("palettes/claude.json");

pub(super) const BUNDLED: &[&str] = &[
    include_str!("palettes/catppuccin.json"),
    include_str!("palettes/tokyonight.json"),
    include_str!("palettes/gruvbox.json"),
    include_str!("palettes/everforest.json"),
    include_str!("palettes/solarized.json"),
    include_str!("palettes/ayu.json"),
    CLAUDE,
];

#[cfg(test)]
mod tests;

#[cfg(test)]
mod contrast_tests;
