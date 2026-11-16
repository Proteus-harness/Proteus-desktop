//! Explicit opt-in window ownership for visual application state.
//!
//! A registered global is cloned from the application default when a window
//! is created. Existing global accessors then address that window's copy only
//! while GPUI executes that window. No focused-window inference is involved.

use std::{
    any::{Any, TypeId},
    cell::Cell,
};

use collections::{FxHashMap, TypeIdHashMap};

use crate::{App, Global, WindowId};

type GlobalMap = TypeIdHashMap<Box<dyn Any>>;
type CloneGlobal = fn(&dyn Any) -> Box<dyn Any>;

#[derive(Default)]
pub(super) struct WindowGlobals {
    cloners: TypeIdHashMap<CloneGlobal>,
    windows: FxHashMap<WindowId, WindowValues>,
    read_scope: Cell<Option<WindowId>>,
}

struct WindowValues {
    globals: GlobalMap,
    reduce_motion: bool,
}

impl App {
    /// Make a visual global independent in every native window.
    ///
    /// Call after assigning its application default. Existing and future
    /// windows start with independent clones of that default. Registered
    /// globals use the current *execution* window in all global accessors;
    /// callers outside a window still address the application default.
    /// Windowless entity callbacks must explicitly update their WindowHandle.
    pub fn register_window_global<G: Global + Clone>(&mut self) {
        let key = TypeId::of::<G>();
        if self.window_globals.cloners.contains_key(&key) {
            return;
        }
        let source = self.globals_by_type.get(&key).unwrap_or_else(|| {
            panic!("set the application default before registering a window global")
        });
        let cloner: CloneGlobal = |value| Box::new(value.downcast_ref::<G>().unwrap().clone());
        for values in self.window_globals.windows.values_mut() {
            values.globals.insert(key, cloner(source.as_ref()));
        }
        self.window_globals.cloners.insert(key, cloner);
    }

    /// The window whose construction, drawing, event, or update is executing.
    /// This deliberately does not return the platform's focused window.
    pub fn current_execution_window_id(&self) -> Option<WindowId> {
        self.window_globals
            .read_scope
            .get()
            .or_else(|| self.window_update_stack.last().copied())
    }

    /// Read a registered value owned by a specific live window.
    /// This is useful for a windowless owner callback; mutation should run
    /// through that owner's WindowHandle so its visual effects are scoped too.
    pub fn window_global<G: Global>(&self, window_id: WindowId) -> Option<&G> {
        crate::fast::dependencies::note_global_read(self, TypeId::of::<G>());
        self.window_globals
            .windows
            .get(&window_id)?
            .globals
            .get(&TypeId::of::<G>())?
            .downcast_ref()
    }

    pub(crate) fn initialize_window_globals(&mut self, id: WindowId) {
        let globals = self
            .window_globals
            .cloners
            .iter()
            .filter_map(|(key, clone)| {
                self.globals_by_type
                    .get(key)
                    .map(|value| (*key, clone(value.as_ref())))
            })
            .collect();
        self.window_globals.windows.insert(
            id,
            WindowValues {
                globals,
                reduce_motion: self.reduce_motion,
            },
        );
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn clear_registered_window_globals(&mut self) {
        self.window_globals.cloners.clear();
        for values in self.window_globals.windows.values_mut() {
            values.globals.clear();
        }
    }

    pub(crate) fn remove_window_globals(&mut self, id: WindowId) {
        self.window_globals.windows.remove(&id);
    }

    pub(crate) fn global_owner<G: Global>(&self) -> Option<WindowId> {
        self.window_globals
            .cloners
            .contains_key(&TypeId::of::<G>())
            .then(|| self.current_execution_window_id())
            .flatten()
    }

    pub(crate) fn global_map<G: Global>(&self) -> &GlobalMap {
        match self.global_owner::<G>() {
            Some(id) => {
                &self
                    .window_globals
                    .windows
                    .get(&id)
                    .expect("executing window has no visual state")
                    .globals
            }
            None => &self.globals_by_type,
        }
    }

    pub(crate) fn global_map_mut<G: Global>(&mut self) -> &mut GlobalMap {
        self.global_map_for_owner_mut(self.global_owner::<G>())
    }

    pub(crate) fn global_map_for_owner_mut(&mut self, owner: Option<WindowId>) -> &mut GlobalMap {
        match owner {
            Some(id) => {
                &mut self
                    .window_globals
                    .windows
                    .get_mut(&id)
                    .expect("executing window has no visual state")
                    .globals
            }
            None => &mut self.globals_by_type,
        }
    }

    pub(crate) fn window_reduce_motion(&self) -> bool {
        self.current_execution_window_id()
            .and_then(|id| self.window_globals.windows.get(&id))
            .map_or(self.reduce_motion, |values| values.reduce_motion)
    }

    pub(crate) fn set_window_reduce_motion(&mut self, value: bool) -> bool {
        let current = match self.current_execution_window_id() {
            Some(id) => {
                &mut self
                    .window_globals
                    .windows
                    .get_mut(&id)
                    .expect("executing window has no visual state")
                    .reduce_motion
            }
            None => &mut self.reduce_motion,
        };
        let changed = *current != value;
        *current = value;
        changed
    }

    /// Immutable window reads carry ownership too. The guard only selects a
    /// typed map; it never moves or swaps application-global values.
    pub(crate) fn with_window_global_read_scope<R>(
        &self,
        id: WindowId,
        callback: impl FnOnce(&App) -> R,
    ) -> R {
        struct ReadScope<'a> {
            slot: &'a Cell<Option<WindowId>>,
            previous: Option<WindowId>,
        }
        impl Drop for ReadScope<'_> {
            fn drop(&mut self) {
                self.slot.set(self.previous);
            }
        }
        let slot = &self.window_globals.read_scope;
        let _scope = ReadScope {
            slot,
            previous: slot.replace(Some(id)),
        };
        callback(self)
    }

    /// Use the same nested execution context for direct draws and ordinary
    /// window updates. Restore it even when a caller catches a panic.
    pub(crate) fn with_window_global_scope<R>(
        &mut self,
        id: WindowId,
        callback: impl FnOnce(&mut App) -> R,
    ) -> R {
        self.window_update_stack.push(id);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(self)));
        let popped = self.window_update_stack.pop();
        debug_assert_eq!(popped, Some(id));
        match outcome {
            Ok(result) => result,
            Err(panic) => std::panic::resume_unwind(panic),
        }
    }
}
