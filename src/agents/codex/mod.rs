//! Shared native Codex app-server service. RPC methods block: run off the UI thread.
mod catalog;
mod projection;
mod recovery;
mod router;
mod subscriptions;
mod types;

pub(crate) use catalog::CatalogCursor;
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
use subscriptions::Subscriptions;
pub use types::{CatalogEvent, Event, Item, ItemKind, Model, Page, Summary};

#[derive(Clone)]
pub struct CodexService {
    client: Arc<Client>,
    router: Arc<Mutex<Router>>,
    subscriptions: Arc<Subscriptions>,
    models: Arc<Mutex<Option<Vec<Model>>>>,
    catalog_events: Arc<Mutex<Option<async_channel::Receiver<CatalogEvent>>>>,
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
    subscriptions: Arc<Subscriptions>,
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
                let subscription = subscriptions.for_thread(&id);
                let _subscription = subscription.lock().unwrap();
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
        let (catalog_sender, catalog_receiver) = async_channel::bounded(256);
        let routing = router.clone();
        let responder = Arc::downgrade(&client);
        std::thread::Builder::new()
            .name("codex-router".into())
            .spawn(move || {
                while let Ok(event) = events.recv_blocking() {
                    if let proteus_codex_client::Event::Notification { method, params } = &event {
                        if method == "thread/deleted" {
                            if let Some(id) = params.get("threadId").and_then(Value::as_str) {
                                // Bound both the count and aggregate metadata size. Native
                                // ids are UUIDs; malformed peers must not allocate GiB here.
                                if id.len() > 16 * 1024 {
                                    routing.lock().unwrap().disconnect(
                                        "Codex deleted thread id exceeded its limit".into(),
                                    );
                                    if let Some(client) = responder.upgrade() {
                                        client.shutdown();
                                    }
                                    break;
                                }
                                routing.lock().unwrap().deleted(id);
                                // Never hold Router across backpressure: dropping a tab
                                // also needs its mutex. Deletions cannot be coalesced/dropped.
                                if catalog_sender
                                    .send_blocking(CatalogEvent::ThreadDeleted {
                                        id: id.to_owned(),
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            continue;
                        }
                    }
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
            subscriptions: Arc::new(Subscriptions::default()),
            models: Arc::new(Mutex::new(None)),
            catalog_events: Arc::new(Mutex::new(Some(catalog_receiver))),
        })
    }
    pub fn is_connected(&self) -> bool {
        self.client.is_connected() && self.router.lock().unwrap().is_connected()
    }
    /// A single group-wide consumer broadcasts catalog changes to all windows.
    pub fn take_catalog_events(&self) -> Option<async_channel::Receiver<CatalogEvent>> {
        self.catalog_events.lock().unwrap().take()
    }
    pub fn list(&self, cwd: &Path, cursor: Option<&str>) -> Result<Page<Summary>, Error> {
        let response = self.client.request(
            "thread/list",
            json!({"cwd":cwd,"cursor":cursor,"limit":50,
            "sortKey":"updated_at","sortDirection":"desc","useStateDbOnly":true,"modelProviders":[],
            "sourceKinds":["cli","vscode","exec","appServer","subAgentThreadSpawn"]}),
        )?;
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
        // Catalogs belong to this app-server's configuration. Reopening a chat
        // must not repeat a potentially remote model discovery. Keep concurrent
        // first reads single-flight, and retry failures without caching them.
        let mut cached = self.models.lock().unwrap();
        if let Some(models) = cached.as_ref() {
            return Ok(models.clone());
        }
        let models = self.load_models()?;
        *cached = Some(models.clone());
        Ok(models)
    }

    fn load_models(&self) -> Result<Vec<Model>, Error> {
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
        let subscription = self.subscriptions.for_thread(id);
        let _subscription = subscription.lock().unwrap();
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
        let subscription = self.subscriptions.for_thread(id);
        let _subscription = subscription.lock().unwrap();
        self.router.lock().unwrap().suspend(id);
        // Some app-server versions stall teardown while this connection is
        // subscribed. Release it first, keeping other threads independent.
        let result = self
            .client
            .request("thread/unsubscribe", json!({"threadId":id}))
            .and_then(|_| self.client.request("thread/delete", json!({"threadId":id})))
            .map(|_| ());
        if let Err(error) = &result {
            // Only a remote rejection establishes a known outcome. A timeout
            // does not cancel deletion and must never recreate that thread.
            let recovered = matches!(error, Error::Remote(_)) && self.recover_subscription(id);
            if !recovered {
                self.router.lock().unwrap().disconnect_thread(
                    id,
                    format!("Ошибка удаления: {error}. Закройте вкладку и откройте чат заново, если он остался в списке."),
                );
            }
        }
        result
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
            && self
                .0
                .router
                .lock()
                .unwrap()
                .connected_generation(self.thread_id(), self.0.generation)
    }
    pub fn send(
        &self,
        text: &str,
        model: Option<&str>,
        effort: Option<&str>,
    ) -> Result<String, Error> {
        // A check before I/O alone races unsubscribe/delete. Serialize the
        // complete turn/start with subscription changes, rejecting new input
        // immediately instead of queuing it behind a possibly slow deletion.
        let subscription = self.0.subscriptions.for_thread(self.thread_id());
        let _subscription = subscription.try_lock().map_err(|_| {
            Error::Protocol("Codex thread subscription is changing; try sending again".into())
        })?;
        self.ensure_connected()?;
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
        self.ensure_connected()?;
        self.0.client.request(
            "turn/interrupt",
            json!({"threadId":self.thread_id(),"turnId":turn_id}),
        )?;
        Ok(())
    }
    pub fn respond(&self, id: Value, result: Result<Value, RpcError>) -> Result<(), Error> {
        self.ensure_connected()?;
        self.0.client.respond(id, result)
    }
    pub fn history(&self, cursor: Option<&str>) -> Result<Page<Item>, Error> {
        self.ensure_connected()?;
        Self::load_history(&self.0.client, self.thread_id(), cursor)
    }
    fn ensure_connected(&self) -> Result<(), Error> {
        if self.is_connected() {
            Ok(())
        } else {
            Err(Error::Disconnected(
                "Codex thread subscription closed".into(),
            ))
        }
    }
    fn load_history(
        client: &Client,
        thread_id: &str,
        cursor: Option<&str>,
    ) -> Result<Page<Item>, Error> {
        let response = client.request(
            "thread/items/list",
            json!({"threadId":thread_id,"cursor":cursor,"limit":50,"sortDirection":"desc"}),
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
