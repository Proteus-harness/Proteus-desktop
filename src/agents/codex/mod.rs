//! Shared native Codex app-server service. RPC methods block: run off the UI thread.
mod projection;
mod router;
mod types;

use proteus_codex_client::{Client, Launch};
pub use proteus_codex_client::{Error, RpcError};
pub use router::Events;
use router::Router;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
pub use types::{Event, Item, ItemKind, Model, Page, Summary};

#[derive(Clone)]
pub struct CodexService {
    client: Arc<Client>,
    router: Arc<Mutex<Router>>,
    subscriptions: Arc<Mutex<()>>,
}

pub struct Opened {
    pub handle: ThreadHandle,
    pub events: Events,
    pub metadata: Summary,
    pub history: Page<Item>,
}

#[derive(Clone)]
pub struct ThreadHandle(Arc<Lease>);
struct Lease {
    client: Arc<Client>,
    router: Arc<Mutex<Router>>,
    thread_id: String,
    generation: usize,
    current_turn: Arc<Mutex<Option<String>>>,
    subscriptions: Arc<Mutex<()>>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.router
            .lock()
            .unwrap()
            .unregister(&self.thread_id, self.generation);
        let client = self.client.clone();
        let id = self.thread_id.clone();
        let subscriptions = self.subscriptions.clone();
        let router = self.router.clone();
        // Closing a tab unsubscribes; it never interrupts the agent's turn.
        // A short bounded request cannot retain the owned process indefinitely.
        let _ = std::thread::Builder::new()
            .name("codex-unsubscribe".into())
            .spawn(move || {
                // Serialize unsubscribe with resume: a delayed old tab drop must
                // never unsubscribe a newly reopened subscription.
                let _subscription = subscriptions.lock().unwrap();
                if router.lock().unwrap().contains(&id) {
                    return;
                }
                let _ = client.request_timeout(
                    "thread/unsubscribe",
                    json!({"threadId":id}),
                    Duration::from_secs(2),
                );
            });
    }
}

impl CodexService {
    pub fn spawn(program: PathBuf, cwd: PathBuf) -> Result<Self, Error> {
        Self::launch(Launch::codex(program, cwd))
    }
    /// Explicit launch also allows deterministic synthetic peers in repository tests.
    pub fn launch(launch: Launch) -> Result<Self, Error> {
        let client = Arc::new(Client::spawn(launch)?);
        let events = client
            .take_events()
            .ok_or_else(|| Error::Protocol("Codex event stream already owned".into()))?;
        let router = Arc::new(Mutex::new(Router::default()));
        let routing = router.clone();
        let responder = Arc::downgrade(&client);
        std::thread::Builder::new()
            .name("codex-router".into())
            .spawn(move || {
                while let Ok(event) = events.recv_blocking() {
                    let (reject, connected) = {
                        let mut router = routing.lock().unwrap();
                        let reject = router.raw(event);
                        (reject, router.is_connected())
                    };
                    if let Some(client) = responder.upgrade() {
                        if let Some(id) = reject {
                            let _ = client.respond(id, Err(unsupported_request()));
                        }
                        if !connected {
                            client.shutdown();
                        }
                    }
                }
                routing
                    .lock()
                    .unwrap()
                    .disconnect("Codex app-server connection closed".into());
            })
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(Self {
            client,
            router,
            subscriptions: Arc::new(Mutex::new(())),
        })
    }
    pub fn is_connected(&self) -> bool {
        self.client.is_connected() && self.router.lock().unwrap().is_connected()
    }
    pub fn list(&self, cwd: &Path, cursor: Option<&str>) -> Result<Page<Summary>, Error> {
        let response=self.client.request("thread/list",json!({"cwd":cwd,"cursor":cursor,"limit":50,
            "sortKey":"updated_at","sortDirection":"desc","useStateDbOnly":true,"modelProviders":[]}))?;
        let data = array(&response, "data")?
            .iter()
            .filter_map(projection::summary)
            .collect();
        Ok(Page {
            data,
            next_cursor: cursor_of(&response),
        })
    }
    pub fn models(&self) -> Result<Vec<Model>, Error> {
        let mut models = Vec::new();
        let mut cursor = None;
        // A malformed peer cannot force unbounded catalog pagination.
        for _ in 0..8 {
            let response = self.client.request(
                "model/list",
                json!({"cursor":cursor,"limit":100,"includeHidden":false}),
            )?;
            models.extend(
                array(&response, "data")?
                    .iter()
                    .filter_map(projection::model),
            );
            let next = cursor_of(&response);
            if next.is_none() {
                return Ok(models);
            }
            if next == cursor {
                return Err(Error::Protocol("Codex model cursor did not advance".into()));
            }
            cursor = next;
        }
        Err(Error::Protocol(
            "Codex model catalog exceeded its page limit".into(),
        ))
    }
    pub fn new_thread(&self, cwd: &Path) -> Result<Opened, Error> {
        // The server allocates the opaque id. Keep early events until that id is known.
        self.router.lock().unwrap().starting += 1;
        let response = self.client.request(
            "thread/start",
            json!({"cwd":cwd,"historyMode":"paginated","experimentalRawEvents":false}),
        );
        let result = response.and_then(|response| self.open_response(response));
        let rejected = self.router.lock().unwrap().finish_start();
        for id in rejected {
            let _ = self.client.respond(id, Err(unsupported_request()));
        }
        result
    }
    pub fn resume(&self, id: &str) -> Result<Opened, Error> {
        let _subscription = self.subscriptions.lock().unwrap();
        let (events, current_turn, generation) = self.router.lock().unwrap().register(id)?;
        let handle = self.handle(id, current_turn, generation);
        let response = self
            .client
            .request("thread/resume", json!({"threadId":id,"excludeTurns":true}))?;
        let metadata = metadata(&response)?;
        if metadata.id != id {
            return Err(Error::Protocol("Codex resumed a different thread".into()));
        }
        if metadata.active {
            let turns = self.client.request(
                "thread/turns/list",
                json!({
                    "threadId": id, "limit": 1, "sortDirection": "desc", "itemsView": "notLoaded"
                }),
            )?;
            let current = array(&turns, "data")?
                .first()
                .filter(|turn| turn.get("status").and_then(Value::as_str) == Some("inProgress"))
                .and_then(|turn| turn.get("id").and_then(Value::as_str))
                .map(str::to_owned);
            self.router.lock().unwrap().seed_turn(id, current);
        }
        // Legacy threads can still be paginated through items/list. Never request full turns.
        let history =
            handle.history(response.get("itemsBackwardsCursor").and_then(Value::as_str))?;
        Ok(Opened {
            handle,
            events,
            metadata,
            history,
        })
    }
    fn open_response(&self, response: Value) -> Result<Opened, Error> {
        let metadata = metadata(&response)?;
        let (events, current_turn, generation) =
            self.router.lock().unwrap().register(&metadata.id)?;
        let handle = self.handle(&metadata.id, current_turn, generation);
        // New threads are empty; live notifications already queued remain authoritative.
        Ok(Opened {
            handle,
            events,
            metadata,
            history: Page {
                data: Vec::new(),
                next_cursor: None,
            },
        })
    }
    fn handle(
        &self,
        id: &str,
        current_turn: Arc<Mutex<Option<String>>>,
        generation: usize,
    ) -> ThreadHandle {
        ThreadHandle(Arc::new(Lease {
            client: self.client.clone(),
            router: self.router.clone(),
            thread_id: id.into(),
            generation,
            current_turn,
            subscriptions: self.subscriptions.clone(),
        }))
    }
    pub fn delete_thread(&self, id: &str) -> Result<(), Error> {
        self.client
            .request("thread/delete", json!({"threadId":id}))?;
        Ok(())
    }
}

impl ThreadHandle {
    pub fn thread_id(&self) -> &str {
        &self.0.thread_id
    }
    pub fn current_turn(&self) -> Option<String> {
        self.0.current_turn.lock().unwrap().clone()
    }
    pub fn is_connected(&self) -> bool {
        self.0.client.is_connected()
    }
    pub fn send(
        &self,
        text: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<String, Error> {
        {
            let mut current = self.0.current_turn.lock().unwrap();
            if current.is_some() {
                return Err(Error::Protocol("Codex turn is already running".into()));
            }
            *current = Some(String::new());
        }
        let response = self.0.client.request(
            "turn/start",
            json!({"threadId":self.thread_id(),
            "input":[{"type":"text","text":text}],"model":model,"effort":effort}),
        );
        let result = response.and_then(|r| {
            r.pointer("/turn/id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| Error::Protocol("Codex turn/start omitted turn id".into()))
        });
        let mut current = self.0.current_turn.lock().unwrap();
        if current.as_deref() == Some("") {
            *current = result.as_ref().ok().cloned();
        }
        result
    }
    pub fn interrupt(&self, turn_id: &str) -> Result<(), Error> {
        self.0.client.request(
            "turn/interrupt",
            json!({"threadId":self.thread_id(),"turnId":turn_id}),
        )?;
        Ok(())
    }
    pub fn respond(&self, id: Value, result: Result<Value, RpcError>) -> Result<(), Error> {
        self.0.client.respond(id, result)
    }
    pub fn history(&self, cursor: Option<&str>) -> Result<Page<Item>, Error> {
        let response = self.0.client.request(
            "thread/items/list",
            json!({"threadId":self.thread_id(),"cursor":cursor,"limit":50,"sortDirection":"desc"}),
        )?;
        let mut data: Vec<_> = array(&response, "data")?
            .iter()
            .filter_map(|entry| {
                projection::item(
                    &entry["item"],
                    entry
                        .get("turnId")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                    true,
                    entry.get("startedAtMs").and_then(Value::as_i64),
                )
            })
            .collect();
        data.reverse();
        Ok(Page {
            data,
            next_cursor: cursor_of(&response),
        })
    }
}

fn unsupported_request() -> RpcError {
    RpcError {
        code: -32601,
        message: "This Codex client does not support this server request".into(),
        data: None,
    }
}

fn metadata(response: &Value) -> Result<Summary, Error> {
    let mut metadata = projection::summary(&response["thread"])
        .ok_or_else(|| Error::Protocol("Codex omitted thread metadata".into()))?;
    metadata.model = response
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(metadata.model);
    metadata.effort = response
        .get("reasoningEffort")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(metadata.effort);
    Ok(metadata)
}
fn array<'a>(value: &'a Value, key: &str) -> Result<&'a Vec<Value>, Error> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Protocol(format!("Codex omitted {key} array")))
}
fn cursor_of(value: &Value) -> Option<String> {
    value
        .get("nextCursor")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

#[cfg(test)]
mod tests;
