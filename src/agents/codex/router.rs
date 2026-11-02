use super::{Event, Item, projection};
use proteus_codex_client::Event as RawEvent;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

const MAX_QUEUED_BYTES: usize = 16 * 1024 * 1024;
const MAX_ROUTES: usize = 32;
const ROUTE_CAPACITY: usize = 128;

struct Envelope {
    event: Event,
    bytes: usize,
    budget: Arc<AtomicUsize>,
}
impl Drop for Envelope {
    fn drop(&mut self) {
        self.budget.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// One consumer, moved with the chat when its tab is detached.
pub struct Events {
    receiver: async_channel::Receiver<Envelope>,
    reason: Arc<Mutex<Option<String>>>,
}
impl Events {
    pub async fn recv(&self) -> Result<Event, async_channel::RecvError> {
        self.receiver.recv().await.map(|mut value| {
            std::mem::replace(&mut value.event, Event::Disconnected(String::new()))
        })
    }
    #[cfg(test)]
    pub fn recv_blocking(&self) -> Result<Event, async_channel::RecvError> {
        self.receiver.recv_blocking().map(|mut value| {
            std::mem::replace(&mut value.event, Event::Disconnected(String::new()))
        })
    }
    pub fn try_recv(&self) -> Result<Event, async_channel::TryRecvError> {
        self.receiver.try_recv().map(|mut value| {
            std::mem::replace(&mut value.event, Event::Disconnected(String::new()))
        })
    }
    #[cfg(test)]
    pub fn is_closed(&self) -> bool {
        self.receiver.is_closed()
    }
    pub fn disconnect_reason(&self) -> Option<String> {
        self.reason.lock().unwrap().clone()
    }
}

pub(super) struct Route {
    sender: async_channel::Sender<Envelope>,
    pub current_turn: Arc<Mutex<Option<String>>>,
    reason: Arc<Mutex<Option<String>>>,
    generation: usize,
    suspended: bool,
    saw_turn_event: bool,
    resync_items: Option<HashSet<String>>,
}
struct Early {
    thread_id: String,
    event: Event,
    bytes: usize,
}
#[derive(Default)]
pub(super) struct Router {
    routes: HashMap<String, Route>,
    early: VecDeque<Early>,
    deleted_starting: HashSet<String>,
    early_bytes: usize,
    pub starting: usize,
    generation: usize,
    budget: Arc<AtomicUsize>,
    disconnected: Option<String>,
}

impl Router {
    pub fn register(
        &mut self,
        id: &str,
    ) -> Result<(Events, Arc<Mutex<Option<String>>>, usize), proteus_codex_client::Error> {
        use proteus_codex_client::Error;
        if let Some(reason) = &self.disconnected {
            return Err(Error::Disconnected(reason.clone()));
        }
        if self.deleted_starting.contains(id) {
            return Err(Error::Disconnected(
                "Codex thread was deleted while opening".into(),
            ));
        }
        if self.routes.contains_key(id) {
            return Err(Error::Protocol("Codex thread is already open".into()));
        }
        if self.routes.len() >= MAX_ROUTES {
            return Err(Error::Overloaded);
        }
        self.generation += 1;
        let generation = self.generation;
        let (sender, receiver) = async_channel::bounded(ROUTE_CAPACITY);
        let current_turn = Arc::new(Mutex::new(None));
        let reason = Arc::new(Mutex::new(None));
        self.routes.insert(
            id.into(),
            Route {
                sender,
                current_turn: current_turn.clone(),
                reason: reason.clone(),
                generation,
                suspended: false,
                saw_turn_event: false,
                resync_items: None,
            },
        );
        let mut retained = VecDeque::new();
        while let Some(event) = self.early.pop_front() {
            if event.thread_id == id {
                self.early_bytes -= event.bytes;
                self.deliver(id, event.event);
            } else {
                retained.push_back(event);
            }
        }
        self.early = retained;
        Ok((Events { receiver, reason }, current_turn, generation))
    }
    pub fn seed_turn(&mut self, id: &str, turn: Option<String>) {
        if let Some(route) = self.routes.get_mut(id) {
            if !route.saw_turn_event {
                *route.current_turn.lock().unwrap() = turn;
            }
        }
    }
    pub fn contains(&self, id: &str) -> bool {
        self.routes.contains_key(id)
    }
    pub fn contains_generation(&self, id: &str, generation: usize) -> bool {
        self.routes
            .get(id)
            .is_some_and(|route| route.generation == generation && !route.sender.is_closed())
    }
    pub fn connected_generation(&self, id: &str, generation: usize) -> bool {
        self.contains_generation(id, generation)
            && self.routes.get(id).is_some_and(|route| !route.suspended)
    }
    pub fn suspend(&mut self, id: &str) {
        let Some(route) = self.routes.get_mut(id) else {
            return;
        };
        route.suspended = true;
        self.deliver(id, Event::SubscriptionSuspended);
    }
    pub fn begin_resync(&mut self, id: &str) -> Option<usize> {
        let route = self.routes.get_mut(id)?;
        let generation = route.generation;
        route.suspended = true;
        route.saw_turn_event = false;
        route.resync_items = Some(HashSet::new());
        self.deliver(id, Event::ResyncStarted);
        self.contains_generation(id, generation)
            .then_some(generation)
    }
    pub fn finish_resync(
        &mut self,
        id: &str,
        generation: usize,
        turn: Option<String>,
        mut history: Vec<Item>,
    ) -> bool {
        let Some(route) = self
            .routes
            .get_mut(id)
            .filter(|route| route.generation == generation)
        else {
            return false;
        };
        if !route.saw_turn_event {
            *route.current_turn.lock().unwrap() = turn;
        }
        if let Some(changed) = route.resync_items.take() {
            history.retain(|item| !changed.contains(&item.id));
        }
        let current_turn = route.current_turn.lock().unwrap().clone();
        route.suspended = false;
        self.deliver(
            id,
            Event::Resynced {
                current_turn,
                history,
            },
        );
        self.contains_generation(id, generation)
    }
    pub fn deleted(&mut self, id: &str) {
        self.disconnect_thread(id, "Чат удалён".into());
        if self.starting > 0 {
            if self.deleted_starting.len() >= ROUTE_CAPACITY && !self.deleted_starting.contains(id)
            {
                self.disconnect("Codex startup deletion buffer exceeded its limit".into());
                return;
            }
            self.deleted_starting.insert(id.into());
        }
        self.early.retain(|early| {
            if early.thread_id == id {
                self.early_bytes -= early.bytes;
                false
            } else {
                true
            }
        });
    }
    pub fn unregister(&mut self, id: &str, generation: usize) {
        if self
            .routes
            .get(id)
            .is_some_and(|r| r.generation == generation)
        {
            self.routes.remove(id);
        }
    }

    pub fn disconnect_thread(&mut self, id: &str, reason: String) {
        if let Some(route) = self.routes.remove(id) {
            *route.reason.lock().unwrap() = Some(reason);
            route.sender.close();
        }
    }
    pub fn finish_start(&mut self) -> Vec<Value> {
        self.starting = self.starting.saturating_sub(1);
        if self.starting == 0 {
            self.early_bytes = 0;
            self.deleted_starting.clear();
            return self
                .early
                .drain(..)
                .filter_map(|early| match early.event {
                    Event::PendingRequest { id, .. } => Some(id),
                    _ => None,
                })
                .collect();
        }
        Vec::new()
    }
    pub fn is_connected(&self) -> bool {
        self.disconnected.is_none()
    }
    /// Return unsupported server request ids for an explicit JSON-RPC rejection.
    pub fn raw(&mut self, event: RawEvent) -> Option<Value> {
        let (thread_id, event) = match event {
            RawEvent::Disconnected(reason) => {
                self.disconnect(reason);
                return None;
            }
            RawEvent::Notification { method, params } => {
                let Some(id) = thread_id(&params) else {
                    return None;
                };
                let Some(event) = projection::notification(&method, &params) else {
                    return None;
                };
                (id.to_owned(), event)
            }
            RawEvent::Request { id, method, params } => {
                let Some(thread) = thread_id(&params) else {
                    // Never auto-approve or strand a global interaction. The
                    // service rejects this exact opaque id, keeping routes live.
                    return Some(id);
                };
                (
                    thread.to_owned(),
                    Event::PendingRequest { id, method, params },
                )
            }
        };
        let request_id = match &event {
            Event::PendingRequest { id, .. } => Some(id.clone()),
            _ => None,
        };
        if self.routes.contains_key(&thread_id) {
            self.deliver(&thread_id, event);
        } else if self.starting > 0 {
            let bytes = event_bytes(&event);
            if self.early.len() >= ROUTE_CAPACITY
                || self.early_bytes.saturating_add(bytes) > MAX_QUEUED_BYTES
            {
                self.disconnect("Codex startup event buffer exceeded its limit".into());
                return request_id;
            }
            self.early_bytes += bytes;
            self.early.push_back(Early {
                thread_id,
                event,
                bytes,
            });
        } else {
            return request_id;
        }
        None
    }
    fn deliver(&mut self, id: &str, event: Event) {
        let Some(route) = self.routes.get_mut(id) else {
            return;
        };
        if let Some(changed) = route.resync_items.as_mut() {
            let item_id = match &event {
                Event::ItemUpsert(item) => Some(&item.id),
                Event::TextDelta { id, .. }
                | Event::ReasoningDelta { id, .. }
                | Event::ToolOutputDelta { id, .. } => Some(id),
                _ => None,
            };
            if let Some(item_id) = item_id {
                // Recovery performs bounded RPCs. Bound even a continuously
                // consumed stream of unique ids while those RPCs are pending.
                if changed.len() >= ROUTE_CAPACITY && !changed.contains(item_id) {
                    self.disconnect_thread(
                        id,
                        "Codex changed too many items while resynchronizing; reopen this chat"
                            .into(),
                    );
                    return;
                }
                changed.insert(item_id.clone());
            }
        }
        if matches!(
            &event,
            Event::TurnStarted { .. } | Event::TurnCompleted { .. }
        ) {
            route.saw_turn_event = true;
        }
        match &event {
            Event::TurnStarted { id } => *route.current_turn.lock().unwrap() = Some(id.clone()),
            Event::TurnCompleted { id, .. } => {
                let mut current = route.current_turn.lock().unwrap();
                if current.as_deref() == Some(id) {
                    *current = None;
                }
            }
            _ => {}
        }
        let bytes = event_bytes(&event);
        let reserved = self
            .budget
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                n.checked_add(bytes).filter(|n| *n <= MAX_QUEUED_BYTES)
            })
            .is_ok();
        let failed = if reserved {
            route
                .sender
                .try_send(Envelope {
                    event,
                    bytes,
                    budget: self.budget.clone(),
                })
                .is_err()
        } else {
            true
        };
        if failed {
            *route.reason.lock().unwrap() =
                Some("Codex event consumer fell behind; reopen this chat to resynchronize".into());
            route.sender.close();
            self.routes.remove(id);
        }
    }
    pub fn disconnect(&mut self, reason: String) {
        self.disconnected = Some(reason.clone());
        for route in self.routes.values() {
            *route.reason.lock().unwrap() = Some(reason.clone());
            route.sender.close();
        }
        self.routes.clear();
        self.early.clear();
        self.deleted_starting.clear();
        self.early_bytes = 0;
    }
}

fn thread_id(params: &Value) -> Option<&str> {
    params
        .get("threadId")
        .or_else(|| params.pointer("/thread/id"))
        .or_else(|| params.pointer("/turn/threadId"))
        .and_then(Value::as_str)
}
fn event_bytes(event: &Event) -> usize {
    match event {
        Event::Resynced { history, .. } => history.iter().map(item_bytes).sum::<usize>() + 1024,
        Event::ItemUpsert(item) => item_bytes(item),
        Event::TextDelta { delta, .. }
        | Event::ReasoningDelta { delta, .. }
        | Event::ToolOutputDelta { delta, .. } => delta.len() + 512,
        Event::PendingRequest { params, .. } | Event::Plan(params) | Event::Usage(params) => {
            params.to_string().len() + 1024
        }
        _ => 1024,
    }
}
fn item_bytes(item: &Item) -> usize {
    match &item.kind {
        super::ItemKind::User { text }
        | super::ItemKind::Assistant { text }
        | super::ItemKind::Reasoning { text }
        | super::ItemKind::Plan { text }
        | super::ItemKind::Other { text, .. } => text.len() + 512,
        super::ItemKind::Tool { input, output, .. } => {
            input.to_string().len() + output.len() + 1024
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn authoritative_deletion_before_start_response_cannot_reopen_a_stale_thread() {
        let mut router = Router::default();
        router.starting = 1;
        router.raw(RawEvent::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"a","turn":{"id":"early"}}),
        });
        router.deleted("a");
        assert!(router.register("a").is_err());
        assert!(router.early.is_empty());
        assert_eq!(router.early_bytes, 0);
        router.finish_start();
        assert!(
            router.deleted_starting.is_empty(),
            "temporary tombstones must not grow over process lifetime"
        );
    }
    #[test]
    fn recovery_orders_new_interactions_and_keeps_live_updates_authoritative() {
        let mut router = Router::default();
        let (events, current, generation) = router.register("a").unwrap();
        router.seed_turn("a", Some("old".into()));
        assert_eq!(router.begin_resync("a"), Some(generation));
        router.deliver(
            "a",
            Event::PendingRequest {
                id: json!("new-request"),
                method: "item/tool/requestUserInput".into(),
                params: json!({}),
            },
        );
        router.deliver(
            "a",
            Event::TextDelta {
                id: "changed".into(),
                turn_id: "old".into(),
                delta: " live".into(),
            },
        );
        router.deliver(
            "a",
            Event::TurnCompleted {
                id: "old".into(),
                status: "completed".into(),
                error: None,
            },
        );
        let item = |id: &str| Item {
            id: id.into(),
            turn_id: "old".into(),
            kind: super::super::ItemKind::Assistant {
                text: "snapshot".into(),
            },
            status: "completed".into(),
            timestamp_ms: None,
        };
        assert!(router.finish_resync(
            "a",
            generation,
            Some("old".into()),
            vec![item("changed"), item("historical")]
        ));
        assert_eq!(current.lock().unwrap().as_deref(), None);
        assert_eq!(events.try_recv().unwrap(), Event::ResyncStarted);
        assert!(
            matches!(events.try_recv().unwrap(), Event::PendingRequest { id, .. } if id == json!("new-request"))
        );
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::TextDelta { .. }
        ));
        assert!(matches!(
            events.try_recv().unwrap(),
            Event::TurnCompleted { .. }
        ));
        let Event::Resynced {
            current_turn,
            history,
        } = events.try_recv().unwrap()
        else {
            panic!("missing snapshot");
        };
        assert!(current_turn.is_none());
        assert_eq!(history, vec![item("historical")]);
    }
    #[test]
    fn recovery_cannot_mutate_or_connect_a_new_generation() {
        let mut router = Router::default();
        let (_old, _, old_generation) = router.register("a").unwrap();
        router.begin_resync("a").unwrap();
        router.unregister("a", old_generation);
        let (new, current, new_generation) = router.register("a").unwrap();
        assert!(!router.contains_generation("a", old_generation));
        assert!(router.contains_generation("a", new_generation));
        assert!(!router.finish_resync("a", old_generation, Some("stale".into()), vec![]));
        assert!(current.lock().unwrap().is_none());
        assert!(new.try_recv().is_err());
    }
    #[test]
    fn early_events_arrive_after_registration_and_stale_completion_keeps_new_turn() {
        let mut router = Router::default();
        router.starting = 1;
        router.raw(RawEvent::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"a","turn":{"id":"new"}}),
        });
        let (events, current, _) = router.register("a").unwrap();
        assert!(router.finish_start().is_empty());
        assert_eq!(
            events.try_recv().unwrap(),
            Event::TurnStarted { id: "new".into() }
        );
        router.raw(RawEvent::Notification {
            method: "turn/completed".into(),
            params: json!({"threadId":"a","turn":{"id":"old","status":"completed"}}),
        });
        assert_eq!(current.lock().unwrap().as_deref(), Some("new"));
    }
    #[test]
    fn resumed_snapshot_cannot_resurrect_an_already_completed_turn() {
        let mut router = Router::default();
        let (_events, current, _) = router.register("a").unwrap();
        router.deliver(
            "a",
            Event::TurnCompleted {
                id: "done".into(),
                status: "completed".into(),
                error: None,
            },
        );
        router.seed_turn("a", Some("done".into()));
        assert_eq!(*current.lock().unwrap(), None);
    }
    #[test]
    fn retry_notice_keeps_turn_running_and_accepts_continued_deltas() {
        let mut router = Router::default();
        let (events, current, _) = router.register("a").unwrap();
        router.deliver("a", Event::TurnStarted { id: "turn".into() });
        router.raw(RawEvent::Notification {
            method: "error".into(),
            params: json!({
                "threadId":"a", "turnId":"turn", "willRetry":true,
                "error":{"message":"synthetic retry", "codexErrorInfo":"streamDisconnected"}
            }),
        });
        assert_eq!(current.lock().unwrap().as_deref(), Some("turn"));
        assert_eq!(
            events.try_recv().unwrap(),
            Event::TurnStarted { id: "turn".into() }
        );
        assert!(events.try_recv().is_err());
        router.raw(RawEvent::Notification {
            method: "item/agentMessage/delta".into(),
            params: json!({
                "threadId":"a","turnId":"turn","itemId":"answer","delta":"continued"
            }),
        });
        assert!(
            matches!(events.try_recv().unwrap(), Event::TextDelta { delta, .. } if delta=="continued")
        );
    }
    #[test]
    fn unsupported_global_request_is_rejected_without_poisoning_routes() {
        let mut router = Router::default();
        let (_events, _, _) = router.register("a").unwrap();
        let id = json!("server:opaque:request");
        let rejection = router.raw(RawEvent::Request {
            id: id.clone(),
            method: "attestation/generate".into(),
            params: json!({"nonce":"synthetic"}),
        });
        assert_eq!(rejection, Some(id));
        assert!(router.is_connected());
        assert!(router.contains("a"));
        router.starting = 1;
        router.raw(RawEvent::Request {
            id: json!("unopened"),
            method: "item/tool/requestUserInput".into(),
            params: json!({"threadId":"unopened-thread"}),
        });
        assert_eq!(router.finish_start(), vec![json!("unopened")]);
    }
    #[test]
    fn routes_are_independent_and_closed_receiver_is_released() {
        let mut router = Router::default();
        let (a, _, _) = router.register("a").unwrap();
        let (b, _, _) = router.register("b").unwrap();
        drop(a);
        router.raw(RawEvent::Notification {
            method: "turn/started".into(),
            params: json!({"threadId":"a","turn":{"id":"t"}}),
        });
        assert!(!router.routes.contains_key("a"));
        assert!(b.try_recv().is_err());
    }
    #[test]
    fn overflow_closes_instead_of_silently_losing_events() {
        let mut router = Router::default();
        let (events, _, _) = router.register("a").unwrap();
        for _ in 0..=ROUTE_CAPACITY {
            router.deliver("a", Event::TurnStarted { id: "turn".into() });
        }
        assert!(events.is_closed());
        assert!(events.disconnect_reason().is_some());
    }
}
