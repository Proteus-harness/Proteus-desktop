//! A package's settings surface and chat content can run in different
//! processes. Invalidation crosses host runtimes, retaining their views and
//! source queues; the originating process cannot choose another package.

use super::Runtime;
use crate::declarative::protocol::SurfaceKind;
use gpui_kit::*;
use std::collections::HashSet;
use std::hash::Hash;

/// Rehosting can register one runtime under several host identities. Select
/// each live runtime once, including the sender, without retaining a registry
/// borrow while calling any runtime.
fn targets<'a, I: Eq + Hash, T>(
    extension: &str,
    entries: impl IntoIterator<Item = (&'a str, I, T)>,
) -> Vec<T> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|(owner, identity, runtime)| {
            (owner == extension && seen.insert(identity)).then_some(runtime)
        })
        .collect()
}

pub(super) fn broadcast(extension: &str, cx: &mut App) {
    broadcast_kind(extension, false, cx);
}

pub(super) fn broadcast_surfaces(extension: &str, cx: &mut App) {
    broadcast_kind(extension, true, cx);
}

pub(super) fn runtimes(extension: &str, cx: &App) -> Vec<Entity<Runtime>> {
    let mut runtimes = cx
        .try_global::<super::super::Runtimes>()
        .map(|runtimes| {
            targets(
                extension,
                runtimes.0.iter().filter_map(|((_, owner), runtime)| {
                    let runtime = runtime.upgrade()?;
                    Some((*owner, runtime.entity_id(), runtime))
                }),
            )
        })
        .unwrap_or_default();
    if let Some(owned) = cx.try_global::<super::super::ViewRuntimes>() {
        runtimes.extend(
            owned
                .0
                .values()
                .filter_map(WeakEntity::upgrade)
                .filter(|runtime| runtime.read(cx).extension_id() == extension),
        );
    }
    let mut seen = HashSet::new();
    runtimes.retain(|runtime| seen.insert(runtime.entity_id()));
    runtimes
}

fn broadcast_kind(extension: &str, all: bool, cx: &mut App) {
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
mod tests {
    use super::targets;

    #[test]
    fn targets_include_sender_and_other_hosts_but_exclude_other_extensions_and_aliases() {
        let entries = [
            ("fixture-renderer", 1, ("settings-host", "sender")),
            ("fixture-renderer", 2, ("chat-host", "chat")),
            ("other-renderer", 3, ("chat-host", "other-package")),
            ("fixture-renderer", 2, ("rehosted-chat", "same-process")),
            ("fixture-renderer", 4, ("second-chat-host", "second-chat")),
        ];
        assert_eq!(
            targets("fixture-renderer", entries),
            vec![
                ("settings-host", "sender"),
                ("chat-host", "chat"),
                ("second-chat-host", "second-chat"),
            ]
        );
        assert!(targets("absent-renderer", entries).is_empty());
    }
}
