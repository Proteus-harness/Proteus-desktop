use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Io(String),
    Disconnected(String),
    Protocol(String),
    Overloaded,
    TimedOut,
    Remote(RpcError),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "Codex I/O: {message}"),
            Self::Disconnected(message) => write!(f, "Codex disconnected: {message}"),
            Self::Protocol(message) => write!(f, "Codex protocol: {message}"),
            Self::Overloaded => f.write_str("Codex transport queue is full"),
            Self::TimedOut => f.write_str("Codex request timed out; outcome is unknown"),
            Self::Remote(error) => write!(f, "Codex error {}: {}", error.code, error.message),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, PartialEq)]
pub enum Event {
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Disconnected(String),
}

pub(crate) struct Budget(Arc<AtomicUsize>);

impl Budget {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }
    pub(crate) fn reserve(&self, bytes: usize, limit: usize) -> Result<Permit, Error> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(bytes).filter(|total| *total <= limit)
            })
            .map_err(|_| Error::Overloaded)?;
        Ok(Permit {
            bytes,
            used: self.0.clone(),
        })
    }
}

pub(crate) struct Permit {
    bytes: usize,
    used: Arc<AtomicUsize>,
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub(crate) struct Envelope {
    pub event: Event,
    pub _permit: Permit,
}

/// Deliberately not Clone: competing consumers would steal each other's events.
pub struct EventReceiver(pub(crate) async_channel::Receiver<Envelope>);

impl EventReceiver {
    pub async fn recv(&self) -> Result<Event, async_channel::RecvError> {
        self.0.recv().await.map(|envelope| envelope.event)
    }
    pub fn recv_blocking(&self) -> Result<Event, async_channel::RecvError> {
        self.0.recv_blocking().map(|envelope| envelope.event)
    }
    pub fn try_recv(&self) -> Result<Event, async_channel::TryRecvError> {
        self.0.try_recv().map(|envelope| envelope.event)
    }
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}
