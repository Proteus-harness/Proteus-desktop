//! Ownership travels with Host, including nested content and inline slots.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::declarative::protocol::Server;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(in crate::ui) enum Lifetime {
    #[default]
    Workspace,
    Settings,
    Preview,
}

impl Lifetime {
    pub(in crate::ui) fn session(self) -> bool {
        self == Self::Workspace
    }

    pub(in crate::ui) fn allowed(self, enabled: bool) -> bool {
        !self.session() || enabled
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::ui) struct ViewOwner(u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::ui) struct RuntimeScope {
    pub lifetime: Lifetime,
    pub owner: Option<ViewOwner>,
}

impl RuntimeScope {
    pub(in crate::ui) fn isolated(lifetime: Lifetime) -> Self {
        assert!(!lifetime.session());
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            lifetime,
            owner: Some(ViewOwner(NEXT.fetch_add(1, Ordering::Relaxed))),
        }
    }

    pub(in crate::ui) fn context(self) -> Option<String> {
        self.owner.map(|owner| format!("view-owned-{}", owner.0))
    }

    /// Read production credentials lazily. An isolated tree, including any
    /// renderer chosen by nested Content, never even reads the workspace API.
    pub(in crate::ui) fn connection(
        self,
        production: impl FnOnce() -> (Option<Server>, Option<PathBuf>),
    ) -> (Option<Server>, Option<PathBuf>) {
        if self.lifetime.session() {
            production()
        } else {
            (
                Some(Server {
                    origin: String::new(),
                    token: String::new(),
                }),
                Some(PathBuf::new()),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_renderers_inherit_unique_sessionless_owner_and_empty_hello() {
        for lifetime in [Lifetime::Settings, Lifetime::Preview] {
            let parent = RuntimeScope::isolated(lifetime);
            let child = parent;
            let inline_grandchild = child;
            assert_eq!(parent.owner, inline_grandchild.owner);
            assert_ne!(parent.owner, RuntimeScope::isolated(lifetime).owner);
            assert!(parent.owner.is_some());
            assert_eq!(parent.context(), inline_grandchild.context());
            for scope in [parent, child, inline_grandchild] {
                let (server, workspace) =
                    scope.connection(|| panic!("isolated content read real credentials"));
                assert_eq!(
                    server.unwrap(),
                    Server {
                        origin: String::new(),
                        token: String::new()
                    }
                );
                assert_eq!(workspace, Some(PathBuf::new()));
                assert!(!scope.lifetime.session());
                assert!(scope.lifetime.allowed(false));
            }
        }
        assert!(!Lifetime::Workspace.allowed(false));
        assert!(Lifetime::Workspace.allowed(true));
        assert!(RuntimeScope::default().owner.is_none());
    }
}
