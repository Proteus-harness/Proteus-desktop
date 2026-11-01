use super::{Event, projection};
use proteus_codex_client::Event as RawEvent;
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
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
    saw_turn_event: bool,
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
                saw_turn_event: false,
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
    pub fn unregister(&mut self, id: &str, generation: usize) {
        if self
            .routes
            .get(id)
            .is_some_and(|r| r.generation == generation)
        {
            self.routes.remove(id);
        }
    }
    pub fn finish_start(&mut self) -> Vec<Value> {
        self.starting = self.starting.saturating_sub(1);
        if self.starting == 0 {
            self.early_bytes = 0;
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
        Event::ItemUpsert(item) => match &item.kind {
            super::ItemKind::User { text }
            | super::ItemKind::Assistant { text }
            | super::ItemKind::Reasoning { text }
            | super::ItemKind::Plan { text }
            | super::ItemKind::Other { text, .. } => text.len() + 512,
            super::ItemKind::Tool { input, output, .. } => {
                input.to_string().len() + output.len() + 1024
            }
        },
        Event::TextDelta { delta, .. }
        | Event::ReasoningDelta { delta, .. }
        | Event::ToolOutputDelta { delta, .. } => delta.len() + 512,
        Event::PendingRequest { params, .. } | Event::Plan(params) | Event::Usage(params) => {
            params.to_string().len() + 1024
        }
        _ => 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
