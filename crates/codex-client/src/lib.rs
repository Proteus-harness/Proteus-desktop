//! One owned Codex app-server process, shared by many conversation threads.
//!
//! Requests block and belong on a worker, never the UI thread. There is exactly
//! one event receiver: route its events by thread/turn id at the service layer.
//! This transport neither retries commands nor stores conversation history.

mod process;
mod protocol;

use process::Lifecycle;
pub use protocol::{Error, Event, EventReceiver, RpcError};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc, time::Duration};

#[derive(Clone, Debug)]
pub struct Limits {
    pub max_line_bytes: usize,
    pub max_queued_bytes: usize,
    pub queue_capacity: usize,
    pub max_pending_requests: usize,
    pub request_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_line_bytes: 8 * 1024 * 1024,
            max_queued_bytes: 16 * 1024 * 1024,
            queue_capacity: 128,
            max_pending_requests: 32,
            request_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Launch {
    pub program: PathBuf,
    /// Complete arguments after the executable; useful for a configured wrapper.
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub limits: Limits,
}

impl Launch {
    pub fn codex(program: PathBuf, cwd: PathBuf) -> Self {
        Self {
            program,
            args: vec!["app-server".into(), "--listen".into(), "stdio://".into()],
            cwd,
            limits: Limits::default(),
        }
    }
}

#[derive(Clone)]
pub struct Client(Arc<Lifecycle>);

impl Client {
    /// Starts and initializes the owned child. No authentication files are read
    /// here: Codex uses its own configured authentication and provider settings.
    pub fn spawn(launch: Launch) -> Result<Self, Error> {
        let client = Self(Arc::new(process::spawn(launch)?));
        client.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "proteus-desktop", "title": "Proteus Desktop",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {"experimentalApi": true}
            }),
        )?;
        client
            .0
            .state
            .send_flushed(json!({"method": "initialized", "params": {}}))?;
        Ok(client)
    }

    pub fn take_events(&self) -> Option<EventReceiver> {
        self.0.events.lock().unwrap().take()
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, Error> {
        self.request_timeout(method, params, self.0.state.limits.request_timeout)
    }

    /// Timeout means the outcome is unknown; it never replays the request.
    pub fn request_timeout(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        self.0.state.request(method, params, timeout)
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), Error> {
        self.0
            .state
            .send(json!({"method": method, "params": params}))
    }

    /// Preserves the server's id exactly, including string ids.
    pub fn respond(&self, id: Value, result: Result<Value, RpcError>) -> Result<(), Error> {
        if !id.is_string() && !id.is_i64() && !id.is_u64() {
            return Err(Error::Protocol("unsupported server request id".into()));
        }
        let message = match result {
            Ok(value) => json!({"id": id, "result": value}),
            Err(error) => json!({"id": id, "error": error}),
        };
        self.0.state.send(message)
    }

    pub fn is_connected(&self) -> bool {
        !self
            .0
            .state
            .closed
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Stops only this owned app-server. Turn cancellation is a normal
    /// `turn/interrupt` request, not shutdown.
    pub fn shutdown(&self) {
        self.0
            .state
            .fail(Error::Disconnected("client shut down".into()));
    }
}
