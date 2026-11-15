//! A package's settings surface and chat content can run in different
//! processes. Invalidation crosses host runtimes, retaining their views and
//! source queues; the originating process cannot choose another package.

use super::Runtime;
use crate::declarative::protocol::SurfaceKind;
use gpui_kit::*;

pub(super) fn broadcast(extension: &str, cx: &mut App) {
    broadcast_kind(extension, false, cx);
}

pub(super) fn broadcast_surfaces(extension: &str, cx: &mut App) {
    broadcast_kind(extension, true, cx);
}

pub(super) fn runtimes(extension: &str, cx: &App) -> Vec<Entity<Runtime>> {
    super::super::live_runtimes(Some(extension), cx)
}

fn broadcast_kind(extension: &str, all: bool, cx: &mut App) {
    // Settings can affect a previously visited row whose content surface is
    // currently unloaded. Active-runtime notifications alone cannot reach it.
    crate::ui::extensions::bump_render_generation(cx);
    crate::ui::chat::invalidate_height_cache();
    for runtime in runtimes(extension, cx) {
        runtime.update(cx, |runtime, cx| {
            if all {
                runtime.invalidate_surfaces(cx)
            } else {
                runtime.invalidate_content(cx)
            }
        });
    }
}

impl Runtime {
    pub(super) fn invalidate_content(&mut self, cx: &App) {
        self.invalidate_matching(|surface| surface == SurfaceKind::Content, cx);
    }

    pub(super) fn invalidate_surfaces(&mut self, cx: &App) {
        self.invalidate_matching(
            |surface| {
                matches!(
                    surface,
                    SurfaceKind::Content
                        | SurfaceKind::Preview
                        | SurfaceKind::Badge
                        | SurfaceKind::Popup
                        | SurfaceKind::Panel
                )
            },
            cx,
        );
    }

    pub(super) fn invalidate_preview(&mut self, cx: &App) {
        self.invalidate_matching(|surface| surface == SurfaceKind::Preview, cx);
    }

    fn invalidate_matching(&mut self, include: impl Fn(SurfaceKind) -> bool, cx: &App) {
        // Startup already renders every attached surface. Do not occupy a
        // source queue before a process exists to acknowledge the request.
        if self.process.is_none() {
            return;
        }
        let views: Vec<_> = self
            .views
            .iter()
            .filter(|(_, view)| include(view.surface))
            .map(|(id, _)| id.clone())
            .collect();
        for view in views {
            // Existing render/ContentQueue coalesces repeated invalidations
            // behind an in-flight source without clearing the current tree.
            self.render(&view, cx);
        }
    }
}

#[cfg(test)]
mod live_runtime_registry;

#[cfg(test)]
mod tests {
    #[test]
    fn settings_invalidation_advances_content_generation_without_a_mounted_runtime() {
        let cx = ::gpui::TestAppContext::single();
        cx.update(|cx| {
            let before = crate::ui::extensions::render_generation(cx);
            super::broadcast("unmounted-fixture-renderer", cx);
            let content = crate::ui::extensions::render_generation(cx);
            assert_ne!(content, before);
            super::broadcast_surfaces("unmounted-fixture-renderer", cx);
            assert_ne!(crate::ui::extensions::render_generation(cx), content);
        });
    }
}
