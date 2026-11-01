use crate::protocol::{Budget, Envelope, Permit};
use crate::{Error, Event, EventReceiver, Launch, Limits, RpcError};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

type Reply = mpsc::SyncSender<ReplyEnvelope>;
struct ReplyEnvelope {
    result: Result<Value, Error>,
    _permit: Option<Permit>,
}
struct Frame {
    bytes: Vec<u8>,
    _permit: Permit,
    flushed: Option<mpsc::SyncSender<Result<(), Error>>>,
}

pub(crate) struct State {
    pub limits: Limits,
    pub closed: AtomicBool,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Reply>>,
    writer: async_channel::Sender<Frame>,
    events: async_channel::Sender<Envelope>,
    outgoing: Budget,
    incoming: Budget,
    child: Mutex<Option<Child>>,
}

pub(crate) struct Lifecycle {
    pub state: Arc<State>,
    pub events: Mutex<Option<EventReceiver>>,
    threads: Vec<JoinHandle<()>>,
}

impl Drop for Lifecycle {
    fn drop(&mut self) {
        self.state
            .fail(Error::Disconnected("client dropped".into()));
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

pub(crate) fn spawn(launch: Launch) -> Result<Lifecycle, Error> {
    let limits = launch.limits;
    if limits.queue_capacity == 0
        || limits.max_pending_requests == 0
        || limits.max_line_bytes == 0
        || limits.max_queued_bytes == 0
    {
        return Err(Error::Protocol("transport limits must be positive".into()));
    }
    let mut child = Command::new(&launch.program)
        .args(&launch.args)
        .current_dir(&launch.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| Error::Io(error.to_string()))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let (write_tx, write_rx) = async_channel::bounded::<Frame>(limits.queue_capacity);
    let (event_tx, event_rx) = async_channel::bounded(limits.queue_capacity);
    let state = Arc::new(State {
        limits,
        closed: AtomicBool::new(false),
        next_id: AtomicU64::new(1),
        pending: Mutex::new(HashMap::new()),
        writer: write_tx,
        events: event_tx,
        outgoing: Budget::new(),
        incoming: Budget::new(),
        child: Mutex::new(Some(child)),
    });
    let mut lifecycle = Lifecycle {
        state: state.clone(),
        events: Mutex::new(Some(EventReceiver(event_rx))),
        threads: vec![],
    };
    let writer_state = state.clone();
    lifecycle.threads.push(
        thread::Builder::new()
            .name("codex-writer".into())
            .spawn(move || {
                while let Ok(frame) = write_rx.recv_blocking() {
                    if writer_state.closed.load(Ordering::Acquire) {
                        break;
                    }
                    if let Err(error) = stdin.write_all(&frame.bytes).and_then(|_| stdin.flush()) {
                        let error = Error::Io(error.to_string());
                        if let Some(reply) = frame.flushed {
                            let _ = reply.send(Err(error.clone()));
                        }
                        writer_state.fail(error);
                        break;
                    }
                    if let Some(reply) = frame.flushed {
                        let _ = reply.send(Ok(()));
                    }
                }
            })
            .map_err(|error| Error::Io(error.to_string()))?,
    );
    lifecycle.threads.push(
        thread::Builder::new()
            .name("codex-reader".into())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    match read_frame(&mut reader, state.limits.max_line_bytes) {
                        Ok(Some(bytes)) => {
                            if let Err(error) = state.receive(&bytes) {
                                state.fail(error);
                                break;
                            }
                        }
                        Ok(None) => {
                            state.fail(Error::Disconnected("stdout closed".into()));
                            break;
                        }
                        Err(error) => {
                            state.fail(error);
                            break;
                        }
                    }
                }
            })
            .map_err(|error| Error::Io(error.to_string()))?,
    );
    Ok(lifecycle)
}

impl State {
    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, Error> {
        let id = self
            .next_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
            .map_err(|_| Error::Protocol("request id space exhausted".into()))?;
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let mut pending = self.pending.lock().unwrap();
            if self.closed.load(Ordering::Acquire) {
                return Err(disconnected());
            }
            if pending.len() >= self.limits.max_pending_requests {
                return Err(Error::Overloaded);
            }
            pending.insert(id, tx);
        }
        if let Err(error) = self.send(json!({"id": id, "method": method, "params": params})) {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        match rx.recv_timeout(timeout) {
            Ok(reply) => reply.result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                Err(Error::TimedOut)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(disconnected()),
        }
    }

    pub fn send(&self, value: Value) -> Result<(), Error> {
        self.enqueue(value, None)
    }

    pub fn send_flushed(&self, value: Value) -> Result<(), Error> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.enqueue(value, Some(tx))?;
        rx.recv_timeout(self.limits.request_timeout)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => Error::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => disconnected(),
            })?
    }

    fn enqueue(
        &self,
        value: Value,
        flushed: Option<mpsc::SyncSender<Result<(), Error>>>,
    ) -> Result<(), Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(disconnected());
        }
        let mut bytes =
            serde_json::to_vec(&value).map_err(|_| Error::Protocol("cannot encode JSON".into()))?;
        if bytes.len() > self.limits.max_line_bytes {
            return Err(Error::Protocol("outgoing frame exceeds byte limit".into()));
        }
        bytes.push(b'\n');
        let permit = self
            .outgoing
            .reserve(bytes.len(), self.limits.max_queued_bytes)?;
        self.writer
            .try_send(Frame {
                bytes,
                _permit: permit,
                flushed,
            })
            .map_err(|error| match error {
                async_channel::TrySendError::Full(_) => Error::Overloaded,
                async_channel::TrySendError::Closed(_) => disconnected(),
            })
    }

    fn receive(&self, bytes: &[u8]) -> Result<(), Error> {
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|_| Error::Protocol("invalid JSON frame".into()))?;
        let Value::Object(mut object) = value else {
            return Err(Error::Protocol("expected JSON object".into()));
        };
        if let Some(method) = object.remove("method") {
            let method = method
                .as_str()
                .ok_or_else(|| Error::Protocol("invalid method".into()))?
                .to_owned();
            let params = object.remove("params").unwrap_or(Value::Null);
            let event = if let Some(id) = object.remove("id") {
                if !id.is_string() && !id.is_i64() && !id.is_u64() {
                    return Err(Error::Protocol("unsupported server request id".into()));
                }
                Event::Request { id, method, params }
            } else {
                Event::Notification { method, params }
            };
            let permit = self
                .incoming
                .reserve(bytes.len(), self.limits.max_queued_bytes)?;
            // Blocking here would prevent replies/interrupts while approval is pending.
            self.events
                .try_send(Envelope {
                    event,
                    _permit: permit,
                })
                .map_err(|_| Error::Overloaded)?;
            return Ok(());
        }
        let id = object
            .remove("id")
            .ok_or_else(|| Error::Protocol("missing response id".into()))?;
        let result = if let Some(error) = object.remove("error") {
            Err(Error::Remote(
                serde_json::from_value::<RpcError>(error)
                    .map_err(|_| Error::Protocol("invalid RPC error".into()))?,
            ))
        } else if let Some(result) = object.remove("result") {
            Ok(result)
        } else {
            return Err(Error::Protocol("missing RPC result".into()));
        };
        // Late timed-out replies are discarded, never applied to another request.
        if let Some(id) = id.as_u64() {
            if let Some(reply) = self.pending.lock().unwrap().remove(&id) {
                let permit = match self
                    .incoming
                    .reserve(bytes.len(), self.limits.max_queued_bytes)
                {
                    Ok(permit) => permit,
                    Err(error) => {
                        let _ = reply.send(ReplyEnvelope {
                            result: Err(error.clone()),
                            _permit: None,
                        });
                        return Err(error);
                    }
                };
                let _ = reply.send(ReplyEnvelope {
                    result,
                    _permit: Some(permit),
                });
            }
        }
        Ok(())
    }

    pub fn fail(&self, error: Error) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        self.writer.close();
        // The channel closure is authoritative if a full queue cannot fit this marker.
        if let Ok(permit) = self.incoming.reserve(0, self.limits.max_queued_bytes) {
            let _ = self.events.try_send(Envelope {
                event: Event::Disconnected(error.to_string()),
                _permit: permit,
            });
        }
        self.events.close();
        for (_, reply) in self.pending.lock().unwrap().drain() {
            let _ = reply.send(ReplyEnvelope {
                result: Err(error.clone()),
                _permit: None,
            });
        }
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn disconnected() -> Error {
    Error::Disconnected("transport closed".into())
}

/// `read_line`/`read_until` can allocate without limit before finding a newline.
fn read_frame(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>, Error> {
    let mut output = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| Error::Io(error.to_string()))?;
        if available.is_empty() {
            return if output.is_empty() {
                Ok(None)
            } else {
                Err(Error::Protocol("incomplete JSONL frame".into()))
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let used = newline.map_or(available.len(), |index| index + 1);
        let content = newline.unwrap_or(available.len());
        if output.len().saturating_add(content) > limit {
            return Err(Error::Protocol("incoming frame exceeds byte limit".into()));
        }
        output.extend_from_slice(&available[..content]);
        reader.consume(used);
        if newline.is_some() {
            return Ok(Some(output));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reader_limits_before_unbounded_allocation() {
        assert!(read_frame(&mut &b"12345\n"[..], 4).is_err());
        assert_eq!(
            read_frame(&mut &b"1234\nrest"[..], 4).unwrap(),
            Some(b"1234".to_vec())
        );
        assert!(read_frame(&mut &b"{}"[..], 4).is_err());
    }
}
