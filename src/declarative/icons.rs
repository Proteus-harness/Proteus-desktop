//! Icons named by extensions: every bundled Lucide icon by its file name,
//! such as `file-code` or `git-branch`.

use gpui_kit::assets::IconName as Lucide;
use std::collections::HashMap;
use std::sync::LazyLock;

static ICONS: LazyLock<HashMap<String, Lucide>> = LazyLock::new(|| {
    Lucide::ALL
        .iter()
        .map(|icon| {
            let path = icon.path();
            let name = path
                .trim_start_matches("icons/")
                .trim_end_matches(".svg")
                .to_string();
            (name, *icon)
        })
        .collect()
});

pub fn lucide(name: &str) -> Option<Lucide> {
    ICONS.get(name).copied()
}

#[cfg(test)]
mod tests {
    #[test]
    fn finds_icons_by_file_name() {
        assert!(super::lucide("git-branch").is_some());
        assert!(super::lucide("file-code").is_some());
        assert!(super::lucide("no-such-icon").is_none());
    }
}
