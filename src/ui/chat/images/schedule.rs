//! A single fetch/decode budget covers all chats, including closed workers.
use std::collections::VecDeque;
use std::sync::Weak;

use gpui_kit::*;

use super::{Chat, Key, SHARED, Stored};

pub(super) const MAX_RUNNING: usize = 2;
const MAX_PENDING: usize = 512;

pub(super) struct Request {
    pub chat: WeakEntity<Chat>,
    pub owner: Weak<()>,
    pub key: Key,
    pub ticket: u64,
}

#[derive(Default)]
pub(super) struct Queue {
    pub running: usize,
    pending: VecDeque<Request>,
    capacity_wakeup_needed: bool,
}

impl Queue {
    pub fn push(&mut self, request: Request) -> bool {
        self.pending
            .retain(|request| request.owner.strong_count() > 0);
        if self.pending.len() >= MAX_PENDING {
            self.capacity_wakeup_needed = true;
            return false;
        }
        self.pending.push_back(request);
        true
    }

    fn take_capacity_wakeup(&mut self) -> bool {
        if self.capacity_wakeup_needed && self.pending.len() < MAX_PENDING {
            self.capacity_wakeup_needed = false;
            true
        } else {
            false
        }
    }

    fn begin(&mut self) -> bool {
        if self.running >= MAX_RUNNING {
            return false;
        }
        self.running += 1;
        true
    }

    fn next(&mut self) -> Option<Request> {
        if self.running >= MAX_RUNNING {
            return None;
        }
        while let Some(request) = self.pending.pop_front() {
            if request.owner.strong_count() > 0 {
                assert!(self.begin());
                return Some(request);
            }
        }
        None
    }
}

pub(super) fn drain(cx: &mut App) {
    loop {
        let Some(request) = SHARED.with(|shared| shared.borrow_mut().queue.next()) else {
            break;
        };
        let input = request
            .chat
            .update(cx, |chat, _| {
                let current = chat.sent_images.by_id.get(&request.key.id)?;
                if !current.matches(&request.key, request.ticket) {
                    return None;
                }
                let api = chat.api.as_ref()?;
                if api.connection_id() != request.key.connection
                    || chat.session.dir != request.key.dir
                {
                    return None;
                }
                Some(api.clone())
            })
            .ok()
            .flatten();
        let Some(api) = input else {
            SHARED.with(|shared| shared.borrow_mut().queue.running -= 1);
            continue;
        };
        cx.spawn(async move |cx| {
            let key = request.key.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    api.image(&key.dir, &key.id)
                        .and_then(|bytes| super::decode::prepare(bytes, key.edge))
                })
                .await;
            cx.update(|cx| {
                request
                    .chat
                    .update(cx, |chat, cx| {
                        let Some(slot) = chat.sent_images.by_id.get_mut(&request.key.id) else {
                            return;
                        };
                        if !slot.matches(&request.key, request.ticket) {
                            return;
                        }
                        slot.state = match result {
                            Ok(preview) => {
                                let image = SHARED.with(|shared| {
                                    shared.borrow_mut().cache.insert(
                                        request.key,
                                        preview,
                                        &slot.owner,
                                        Some(cx),
                                    )
                                });
                                Stored::Ready(std::sync::Arc::downgrade(&image))
                            }
                            Err(error) => Stored::Failed(format!("{error:#}").into()),
                        };
                        cx.notify();
                    })
                    .ok();
                // Retain the reservation through HTTP, decode, completed result
                // and owner validation. Cancellation never frees an active worker.
                SHARED.with(|shared| shared.borrow_mut().queue.running -= 1);
                drain(cx);
            });
        })
        .detach();
    }
    if SHARED.with(|shared| shared.borrow_mut().queue.take_capacity_wakeup()) {
        // Full-queue rows retry lazily during their next render. One shared
        // wakeup after capacity becomes available avoids a waiter list and
        // cannot spin while the queue remains full. Hidden views retry on open.
        cx.refresh_windows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn one_global_budget_covers_owners_and_stale_active_results() {
        // Owner cancellation removes pending metadata, but an already admitted
        // operation is released only after its actual result has been consumed.
        let mut queue = Queue::default();
        assert!(queue.begin()); // first owner
        assert!(queue.begin()); // another owner
        assert!(!queue.begin());
        let _cancelled_owner = (); // cancellation intentionally does not decrement
        assert!(!queue.begin());
        queue.running -= 1; // first worker completes, even if its owner is gone
        assert!(queue.begin());
        assert_eq!(queue.running, MAX_RUNNING);
    }

    #[::core::prelude::v1::test]
    fn full_queue_waits_then_wakes_once_and_accepts_retry_without_busy_loop() {
        let owner = std::sync::Arc::new(());
        let request = || Request {
            chat: WeakEntity::new_invalid(),
            owner: std::sync::Arc::downgrade(&owner),
            key: Key {
                connection: 1,
                dir: "session".into(),
                id: "image".into(),
                edge: 96,
            },
            ticket: 1,
        };
        let mut queue = Queue::default();
        assert!(queue.begin());
        assert!(queue.begin());
        for _ in 0..MAX_PENDING {
            assert!(queue.push(request()));
        }
        assert!(!queue.push(request()));
        assert!(queue.next().is_none()); // both workers are still running
        assert!(!queue.take_capacity_wakeup()); // no self-refresh while full
        assert!(!queue.take_capacity_wakeup());
        queue.running -= 1; // real worker completion
        assert!(queue.next().is_some()); // frees one pending slot and starts work
        assert!(queue.take_capacity_wakeup());
        assert!(!queue.take_capacity_wakeup()); // one shared wakeup only
        assert!(queue.push(request())); // previously refused row can retry
        assert_eq!(queue.pending.len(), MAX_PENDING);
        assert!(!queue.take_capacity_wakeup());
    }
}
