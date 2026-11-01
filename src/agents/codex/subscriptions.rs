//! Serialize resume/unsubscribe for the same thread without stalling other chats.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

#[derive(Default)]
pub(super) struct Subscriptions(Mutex<HashMap<String, Weak<Mutex<()>>>>);

impl Subscriptions {
    pub fn for_thread(&self, id: &str) -> Arc<Mutex<()>> {
        let mut slots = self.0.lock().unwrap();
        if let Some(slot) = slots.get(id).and_then(Weak::upgrade) {
            return slot;
        }
        // Finished requests keep no per-thread registry allocation alive.
        slots.retain(|_, slot| slot.strong_count() > 0);
        let slot = Arc::new(Mutex::new(()));
        slots.insert(id.to_owned(), Arc::downgrade(&slot));
        slot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_operations_on_the_same_thread_share_a_lock() {
        let slots = Subscriptions::default();
        let first = slots.for_thread("a");
        assert!(Arc::ptr_eq(&first, &slots.for_thread("a")));
        let held = first.lock().unwrap();
        assert!(slots.for_thread("a").try_lock().is_err());
        assert!(slots.for_thread("b").try_lock().is_ok());
        drop(held);
        drop(first);
        let _next = slots.for_thread("c");
        assert_eq!(slots.0.lock().unwrap().len(), 1);
    }
}
