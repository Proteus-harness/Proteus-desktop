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
    let runtimes = cx
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
    for runtime in runtimes {
        runtime.update(cx, |runtime, cx| runtime.invalidate_content(cx));
    }
}

impl Runtime {
    pub(super) fn invalidate_content(&mut self, cx: &App) {
        // Startup already renders every attached surface. Do not occupy a
        // source queue before a process exists to acknowledge the request.
        if self.process.is_none() {
            return;
        }
        let views: Vec<_> = self
            .views
            .iter()
            .filter(|(_, view)| view.surface == SurfaceKind::Content)
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
