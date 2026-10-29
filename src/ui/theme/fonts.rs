// Адаптировано из Zed: crates/settings_ui/src/components/font_picker.rs
//! A single installed-family inventory, shared by settings and preference
//! resolution. No font discovery is performed while rendering chat rows.

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme as _, ThemeConfig};
use gpui_kit::*;

struct Families {
    installed: Vec<SharedString>,
    default_interface: SharedString,
}

impl Global for Families {}

pub(super) fn init(cx: &mut App) {
    let mut installed: Vec<SharedString> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|family| !family.trim().is_empty())
        .map(Into::into)
        .collect();
    installed.sort();
    installed.dedup();
    let default_interface = cx.theme().font_family.clone();
    cx.set_global(Families {
        installed,
        default_interface,
    });
}

pub(in crate::ui) fn installed(cx: &App) -> &[SharedString] {
    &cx.global::<Families>().installed
}

/// A saved family absent on this machine inherits instead of repeatedly
/// sending a missing font through the native shaping fallback chain.
pub(in crate::ui) fn resolve(requested: Option<&str>, cx: &App) -> Option<SharedString> {
    let requested = requested?;
    installed(cx)
        .iter()
        .find(|family| family.as_ref() == requested)
        .cloned()
}

pub(super) fn configure(
    config: Rc<ThemeConfig>,
    requested: Option<&str>,
    cx: &App,
) -> Rc<ThemeConfig> {
    let mut config = config.as_ref().clone();
    config.font_family = Some(
        resolve(requested, cx)
            .or(config.font_family)
            .unwrap_or_else(|| cx.global::<Families>().default_interface.clone()),
    );
    Rc::new(config)
}
