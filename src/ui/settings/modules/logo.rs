// Адаптировано из Zed: crates/extensions_ui/src/components/extension_card.rs
// (bounded package artwork beside the title). Native GPUI owns async loading.
//! Local logos use the normal image loader and a painted-element-owned cache.
//! Changing source or leaving the page drops the cache and pending work.

use std::path::{Path, PathBuf};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::config_builder::{ConfigBuilderPlugin, ConfigBuilderSnapshot};

use crate::ui::prefs;

#[derive(Debug, PartialEq)]
enum Source {
    Native(Lucide),
    Asset(String),
    File(PathBuf),
}

pub(super) fn render(
    plugin: &ConfigBuilderPlugin,
    snapshot: &ConfigBuilderSnapshot,
    cx: &App,
) -> Div {
    let color = cx.theme().muted_foreground;
    let source = plugin.icon.as_deref().and_then(|icon| {
        resolve(
            icon,
            snapshot
                .target_path
                .as_deref()
                .or(snapshot.config_path.as_deref()),
        )
    });
    let content = match source {
        Some(Source::Native(icon)) => Icon::new(icon)
            .size(prefs::scaled(20.))
            .text_color(color)
            .into_any_element(),
        Some(source) => {
            let key = super::hashed_id("module-logo-cache", &(&plugin.id, format!("{source:?}")));
            let image = match source {
                Source::Asset(path) => ImageSource::Resource(Resource::Embedded(path.into())),
                Source::File(path) => ImageSource::from(path),
                Source::Native(_) => unreachable!(),
            };
            image_cache(retain_all(key))
                .size(prefs::scaled(20.))
                .child(
                    img(image)
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .with_loading(move || fallback(color))
                        .with_fallback(move || fallback(color)),
                )
                .into_any_element()
        }
        None => fallback(color),
    };
    h_flex()
        .size(prefs::scaled(36.))
        .flex_none()
        .items_center()
        .justify_center()
        .child(content)
}

fn fallback(color: Hsla) -> AnyElement {
    Icon::new(Lucide::Boxes)
        .size(prefs::scaled(20.))
        .text_color(color)
        .into_any_element()
}

/// The DTO is presentation-only. Resolve names without inspecting a file in
/// render; relative files require the physical config's absolute directory.
fn resolve(icon: &str, config: Option<&str>) -> Option<Source> {
    let icon = icon.trim();
    if icon.is_empty() {
        return None;
    }
    if let Some(name) = icon.strip_prefix("lucide:") {
        return crate::declarative::icons::lucide(name).map(Source::Native);
    }
    if let Some(path) = icon.strip_prefix("asset:") {
        return (!path.is_empty() && !path.contains("://") && !path.starts_with("data:"))
            .then(|| Source::Asset(path.to_owned()));
    }
    if let Some(icon) = crate::declarative::icons::lucide(icon) {
        return Some(Source::Native(icon));
    }
    // Never construct Resource::Uri: logos cannot turn an ordinary settings
    // visit into a provider/network request, including data: and file: URIs.
    if icon.contains("://") || icon.starts_with("data:") || icon.starts_with("file:") {
        return None;
    }
    let path = Path::new(icon);
    if path.is_absolute() {
        return Some(Source::File(path.to_owned()));
    }
    let config = config?;
    // Same reserved marker as Core's physical_config_path; ordinary # names
    // keep their full filename. No Core implementation is imported by UI.
    let physical = config
        .split_once("#agent=")
        .map_or(config, |(file, _)| file);
    let physical = Path::new(physical);
    if !physical.is_absolute() {
        return None;
    }
    Some(Source::File(physical.parent()?.join(path)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn logos_resolve_against_physical_config_and_never_use_cwd_or_remote_uris() {
        assert_eq!(
            resolve("logos/author.svg", Some("/profiles/main.toml#agent=/child")),
            Some(Source::File("/profiles/logos/author.svg".into()))
        );
        assert_eq!(
            resolve("author.png", Some("/profiles/name#literal.toml")),
            Some(Source::File("/profiles/author.png".into()))
        );
        assert_eq!(
            resolve("/assets/author.png", None),
            Some(Source::File("/assets/author.png".into()))
        );
        assert!(resolve("author.svg", None).is_none());
        assert!(resolve("author.svg", Some("relative/config.toml")).is_none());
        for uri in [
            "https://example.com/logo.svg",
            "http://localhost/logo.png",
            "file:///logo.png",
            "data:image/svg+xml,logo",
        ] {
            assert!(resolve(uri, Some("/profiles/main.toml")).is_none(), "{uri}");
        }
        assert!(matches!(
            resolve("git-branch", None),
            Some(Source::Native(_))
        ));
        assert!(matches!(
            resolve("lucide:file-code", None),
            Some(Source::Native(_))
        ));
        assert_eq!(
            resolve("asset:icons/file-code.svg", None),
            Some(Source::Asset("icons/file-code.svg".into()))
        );
        assert!(resolve("lucide:unknown-icon", None).is_none());
        assert!(resolve("asset:https://example.com/logo.svg", None).is_none());
    }
}
