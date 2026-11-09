//! Native layout observations retain only weak scalar readiness tokens.
//! An asynchronous content view owns its token; cached layout can therefore
//! recheck readiness without rendering the view or retaining its widgets.
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

/// Readiness of one content view, initially waiting for its current source.
#[derive(Default)]
struct Readiness {
    ready: Cell<bool>,
    descendants_active: Cell<bool>,
    parent: RefCell<Weak<Readiness>>,
}

#[derive(Clone, Default)]
pub(crate) struct Token(Rc<Readiness>);

impl Token {
    pub(crate) fn set_ready(&self, ready: bool) {
        self.0.ready.set(ready);
    }

    /// A source/error fallback hides its retained nested content views.
    pub(crate) fn set_descendants_active(&self, active: bool) {
        self.0.descendants_active.set(active);
    }

    /// Only a weak dependency is retained. Reject cycles rather than letting
    /// observation walk an invalid content topology forever.
    pub(crate) fn set_parent(&self, parent: &Token) -> bool {
        let mut ancestor = Some(parent.0.clone());
        while let Some(current) = ancestor {
            if Rc::ptr_eq(&self.0, &current) {
                return false;
            }
            ancestor = current.parent.borrow().upgrade();
        }
        *self.0.parent.borrow_mut() = Rc::downgrade(&parent.0);
        true
    }
}

impl Readiness {
    fn effective_ready(&self) -> bool {
        let mut ancestor = self.parent.borrow().upgrade();
        while let Some(current) = ancestor {
            if !current.descendants_active.get() {
                return true;
            }
            ancestor = current.parent.borrow().upgrade();
        }
        self.ready.get()
    }
}

/// Bounded by the content views participating in one resident row's layout.
/// Released views leave dead weak references, removed on the next observation.
#[derive(Clone, Default)]
pub(crate) struct Observation {
    tokens: Vec<Weak<Readiness>>,
    pending: bool,
}

impl Observation {
    pub(crate) fn is_ready(&self) -> bool {
        !self.pending
            && self
                .tokens
                .iter()
                .filter_map(Weak::upgrade)
                .all(|ready| ready.effective_ready())
    }
}

thread_local! {
    static SCOPES: RefCell<Vec<Observation>> = const { RefCell::new(Vec::new()) };
}

struct Scope {
    depth: usize,
    active: bool,
}

impl Scope {
    fn new(previous: &Observation) -> Self {
        let mut observation = previous.clone();
        observation.tokens.retain(|token| token.strong_count() > 0);
        // Anonymous markers belong to this layout only. Retained asynchronous
        // state is represented by the independently updated tokens instead.
        observation.pending = false;
        let depth = SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            let depth = scopes.len();
            scopes.push(observation);
            depth
        });
        Self {
            depth,
            active: true,
        }
    }

    fn finish(mut self) -> Observation {
        let observation = SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            debug_assert_eq!(scopes.len(), self.depth + 1);
            scopes.pop().expect("active native layout observation")
        });
        self.active = false;
        observation
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        if self.active {
            SCOPES.with(|scopes| scopes.borrow_mut().truncate(self.depth));
        }
    }
}

/// Run a layout, retaining live tokens from its previous cached layout.
/// The guard also restores the enclosing scope if the layout unwinds.
pub(crate) fn observe<T>(previous: &Observation, layout: impl FnOnce() -> T) -> (T, Observation) {
    let scope = Scope::new(previous);
    let result = layout();
    (result, scope.finish())
}

/// Register a content view in every containing layout, including ready views
/// that can become pending again without the surrounding row being rendered.
pub(crate) fn track(token: &Token) {
    let token = Rc::downgrade(&token.0);
    SCOPES.with(|scopes| {
        for scope in scopes.borrow_mut().iter_mut() {
            if !scope.tokens.iter().any(|other| other.ptr_eq(&token)) {
                scope.tokens.push(token.clone());
            }
        }
    });
}

/// Mark a temporary synchronous fallback in all containing layouts.
/// Cached asynchronous views should use a token rather than this marker.
pub(crate) fn pending() {
    SCOPES.with(|scopes| {
        for scope in scopes.borrow_mut().iter_mut() {
            scope.pending = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn nested_pending_content_reaches_its_parent_without_tainting_later_layouts() {
        pending();
        let empty = Observation::default();
        let (value, clean) = observe(&empty, || 7);
        assert_eq!(value, 7);
        assert!(clean.is_ready());
        let ((value, inner), outer) = observe(&empty, || {
            observe(&empty, || {
                pending();
                9
            })
        });
        assert_eq!(value, 9);
        assert!(!inner.is_ready());
        assert!(!outer.is_ready());
        assert!(observe(&outer, || ()).1.is_ready());
    }

    #[::core::prelude::v1::test]
    fn cached_layout_rechecks_live_readiness_without_retaining_the_view() {
        let token = Token::default();
        let (_, first) = observe(&Observation::default(), || track(&token));
        assert!(!first.is_ready());
        token.set_ready(true);
        assert!(first.is_ready());
        let (_, cached) = observe(&first, || ());
        token.set_ready(false);
        assert!(!cached.is_ready());
        drop(token);
        assert!(cached.is_ready());
        let (_, pruned) = observe(&cached, || ());
        assert!(pruned.tokens.is_empty());
    }

    #[::core::prelude::v1::test]
    fn tokens_are_deduplicated_and_propagated_to_every_enclosing_scope() {
        let token = Token::default();
        let empty = Observation::default();
        let (_, outer) = observe(&empty, || {
            track(&token);
            let (_, inner) = observe(&empty, || {
                track(&token);
                track(&token);
            });
            assert_eq!(inner.tokens.len(), 1);
            assert!(!inner.is_ready());
        });
        assert_eq!(outer.tokens.len(), 1);
        assert!(!outer.is_ready());
        token.set_ready(true);
        assert!(outer.is_ready());
    }

    #[::core::prelude::v1::test]
    fn parent_source_hides_pending_descendants_without_hiding_its_own_readiness() {
        let parent = Token::default();
        let child = Token::default();
        let grandchild = Token::default();
        assert!(child.set_parent(&parent));
        assert!(grandchild.set_parent(&child));
        parent.set_ready(true);
        parent.set_descendants_active(true);
        child.set_ready(true);
        child.set_descendants_active(true);
        let (_, observation) = observe(&Observation::default(), || {
            track(&parent);
            track(&child);
            track(&grandchild);
        });
        assert!(!observation.is_ready());
        // Selecting the parent's source keeps its nested views alive, but
        // their pending previews no longer determine the displayed height.
        parent.set_descendants_active(false);
        assert!(observation.is_ready());
        parent.set_ready(false);
        assert!(!observation.is_ready());
        parent.set_ready(true);
        parent.set_descendants_active(true);
        assert!(!observation.is_ready());
        grandchild.set_ready(true);
        assert!(observation.is_ready());
    }

    #[::core::prelude::v1::test]
    fn content_parent_links_are_weak_and_reject_cycles() {
        let parent = Token::default();
        let child = Token::default();
        assert!(child.set_parent(&parent));
        assert!(!parent.set_parent(&child));
        assert!(!parent.set_parent(&parent));
        let weak_parent = Rc::downgrade(&parent.0);
        drop(parent);
        assert!(weak_parent.upgrade().is_none());
        let (_, observation) = observe(&Observation::default(), || track(&child));
        assert!(!observation.is_ready());
        child.set_ready(true);
        assert!(observation.is_ready());
    }

    #[::core::prelude::v1::test]
    fn hiding_one_content_branch_does_not_hide_a_pending_sibling() {
        let sibling = Token::default();
        let source_parent = Token::default();
        let hidden_child = Token::default();
        assert!(hidden_child.set_parent(&source_parent));
        source_parent.set_ready(true);
        source_parent.set_descendants_active(false);
        let (_, observation) = observe(&Observation::default(), || {
            track(&sibling);
            track(&source_parent);
            track(&hidden_child);
        });
        assert!(!observation.is_ready());
        sibling.set_ready(true);
        assert!(observation.is_ready());
        source_parent.set_descendants_active(true);
        assert!(!observation.is_ready());
        hidden_child.set_ready(true);
        assert!(observation.is_ready());
    }

    #[::core::prelude::v1::test]
    fn unwinding_a_nested_layout_restores_the_stack_and_keeps_outer_pending() {
        let empty = Observation::default();
        let (_, outer) = observe(&empty, || {
            let failed = std::panic::catch_unwind(|| {
                observe(&Observation::default(), || {
                    pending();
                    panic!("synthetic layout failure");
                });
            });
            assert!(failed.is_err());
            assert!(observe(&Observation::default(), || ()).1.is_ready());
        });
        assert!(!outer.is_ready());
        assert!(observe(&empty, || ()).1.is_ready());
    }
}
