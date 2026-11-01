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
    /// Content in another driver has no Proteus server capability.
    pub content_only: bool,
}

impl RuntimeScope {
    pub(in crate::ui) fn isolated(lifetime: Lifetime) -> Self {
        assert!(!lifetime.session());
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self {
            lifetime,
            owner: Some(ViewOwner(NEXT.fetch_add(1, Ordering::Relaxed))),
            content_only: true,
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
        if self.lifetime.session() && !self.content_only {
            production()
        } else if self.lifetime.session() {
            // The wire already supports credential-free content processes.
            // Preserve the folder, without even asking for a production API.
            let (_, workspace) = production();
            (Some(empty_server()), workspace)
        } else {
            (Some(empty_server()), Some(PathBuf::new()))
        }
    }
}

fn empty_server() -> Server {
    Server {
        origin: String::new(),
        token: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_content_preserves_folder_but_discards_server_credentials() {
        let scope = RuntimeScope {
            content_only: true,
            ..Default::default()
        };
        let (server, workspace) = scope.connection(|| (None, Some("/project".into())));
        assert_eq!(server, Some(empty_server()));
        assert_eq!(workspace, Some("/project".into()));
        assert!(scope.lifetime.session());
    }

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
