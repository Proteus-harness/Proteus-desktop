//! A running extension process: bounded, ordered JSONL over stdio.
//!
//! App-wide budgets cover queued messages and their current delivery/write.
//! Wire bytes are logical limits: parsing, widget allocations and fixed pipe
//! buffers are additional memory, not a promise of an exact process RSS cap.

mod budget;
mod io;
#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::io::{BufReader, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use anyhow::{Context as _, Result};
use budget::{Budget, Permit};

use super::manifest::Installed;
use super::protocol::{self, Incoming, Outgoing};

const LOG_LINES: usize = 20;
const QUEUE_COUNT: usize = 128;
const INCOMING_BYTES: usize = 64 * 1024 * 1024;
const OUTGOING_BYTES: usize = 32 * 1024 * 1024;
/// How long a stopped extension may take to exit before it is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

/// What the reader threads report.
#[derive(Debug)]
pub enum Signal {
    Message(Incoming),
    Invalid(String),
    /// A normal exit or a terminal transport failure; restarting is explicit.
    Exited(String),
}

/// The permit belongs to the delivery, not just to the channel slot. This is
/// deliberately not Clone: two consumers must not steal each other's actions.
pub struct Delivery {
    signal: Signal,
    _reservation: Option<Permit>,
}

impl Delivery {
    pub fn with_signal<R>(self, handler: impl FnOnce(Signal) -> R) -> R {
        let Self {
            signal,
            _reservation,
        } = self;
        let result = handler(signal);
        drop(_reservation);
        result
    }
}

pub struct Signals {
    incoming: async_channel::Receiver<Delivery>,
    failure: async_channel::Receiver<String>,
    state: Arc<State>,
}

impl Signals {
    /// Failure also closes `incoming`, so it wakes a pending receive. Check it
    /// on both sides of receiving to prioritize it over a full data queue.
    pub async fn recv(&self) -> Result<Delivery, async_channel::RecvError> {
        if let Ok(error) = self.failure.try_recv() {
            return Ok(terminal(error));
        }
        let delivery = self.incoming.recv().await;
        self.prioritize_failure(delivery)
    }

    fn prioritize_failure(
        &self,
        delivery: Result<Delivery, async_channel::RecvError>,
    ) -> Result<Delivery, async_channel::RecvError> {
        match self.failure.try_recv() {
            Ok(error) => Ok(terminal(error)),
            Err(_) => delivery,
        }
    }

    #[cfg(test)]
    fn recv_blocking(&self) -> Result<Delivery, async_channel::RecvError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(error) = self.failure.try_recv() {
                return Ok(terminal(error));
            }
            match self.incoming.try_recv() {
                Ok(delivery) => return self.prioritize_failure(Ok(delivery)),
                Err(async_channel::TryRecvError::Closed) => {
                    return self.prioritize_failure(Err(async_channel::RecvError));
                }
                Err(async_channel::TryRecvError::Empty) => {}
            }
            assert!(
                std::time::Instant::now() < deadline,
                "extension fixture timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        self.state.stop(None);
    }
}

fn terminal(message: String) -> Delivery {
    Delivery {
        signal: Signal::Exited(message),
        _reservation: None,
    }
}

struct Frame {
    bytes: Vec<u8>,
    _reservation: Permit,
}

struct State {
    child: Mutex<Option<Child>>,
    closed: AtomicBool,
    incoming: Arc<Budget>,
    outgoing: Arc<Budget>,
    messages: async_channel::Sender<Delivery>,
    // Receivers are used only to release queued allocations at terminal stop.
    queued_messages: async_channel::Receiver<Delivery>,
    writer: async_channel::Sender<Frame>,
    queued_writes: async_channel::Receiver<Frame>,
    failure: async_channel::Sender<String>,
}

impl State {
    fn stop(&self, failure: Option<String>) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(failure) = failure {
            // A separate one-slot control channel never competes with views.
            let _ = self.failure.try_send(failure);
        }
        self.failure.close();
        self.messages.close();
        self.writer.close();
        self.incoming.close(&self.closed);
        while self.queued_messages.try_recv().is_ok() {}
        while self.queued_writes.try_recv().is_ok() {}
        let Some(mut child) = self.child.lock().unwrap().take() else {
            return;
        };
        let group = child.id() as libc::pid_t;
        // SAFETY: spawn creates a separate process group for this child.
        unsafe { libc::killpg(group, libc::SIGTERM) };
        thread::spawn(move || {
            // Keep the group leader unreaped until the final signal. Reaping
            // it first could allow the numeric process-group id to be reused.
            thread::sleep(STOP_TIMEOUT);
            unsafe { libc::killpg(group, libc::SIGKILL) };
            let _ = child.wait();
        });
    }
}

pub struct Process {
    state: Arc<State>,
}

fn shared_budgets() -> (Arc<Budget>, Arc<Budget>) {
    static BUDGETS: OnceLock<(Arc<Budget>, Arc<Budget>)> = OnceLock::new();
    let budgets = BUDGETS.get_or_init(|| {
        (
            Budget::new(QUEUE_COUNT, INCOMING_BYTES),
            Budget::new(QUEUE_COUNT, OUTGOING_BYTES),
        )
    });
    (budgets.0.clone(), budgets.1.clone())
}

impl Process {
    pub fn spawn(extension: &Installed) -> Result<(Self, Signals)> {
        Self::spawn_with_budgets(extension, shared_budgets())
    }

    fn spawn_with_budgets(
        extension: &Installed,
        budgets: (Arc<Budget>, Arc<Budget>),
    ) -> Result<(Self, Signals)> {
        let (program, args) = extension
            .manifest
            .command
            .split_first()
            .context("command is empty")?;
        let mut command = if program.contains('/') && !program.starts_with('/') {
            Command::new(extension.dir.join(program))
        } else {
            Command::new(program)
        };
        command
            .args(args)
            .current_dir(&extension.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        // SAFETY: prctl is async-signal-safe; an extension must not outlive the client.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("cannot start `{program}`"))?;
        let mut stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let (messages, incoming) = async_channel::bounded(QUEUE_COUNT);
        let (writer, lines) = async_channel::bounded::<Frame>(QUEUE_COUNT);
        let (failure, errors) = async_channel::bounded(1);
        let state = Arc::new(State {
            child: Mutex::new(Some(child)),
            closed: AtomicBool::new(false),
            incoming: budgets.0,
            outgoing: budgets.1,
            messages,
            queued_messages: incoming.clone(),
            writer,
            queued_writes: lines.clone(),
            failure,
        });
        let log = Arc::new(Mutex::new(VecDeque::new()));
        {
            let log = log.clone();
            thread::spawn(move || {
                io::read_log(BufReader::new(stderr), |line| {
                    let mut log = log.lock().unwrap();
                    if log.len() == LOG_LINES {
                        log.pop_front();
                    }
                    log.push_back(line);
                })
            });
        }
        {
            let state = state.clone();
            thread::spawn(move || {
                let mut stdout = BufReader::new(stdout);
                loop {
                    match io::read_frame(&mut stdout, &state) {
                        Ok(Some((bytes, reservation))) => {
                            let line = String::from_utf8_lossy(&bytes);
                            if line.trim().is_empty() {
                                continue;
                            }
                            let signal = match protocol::parse(line.trim_end_matches('\r')) {
                                Ok(message) => Signal::Message(message),
                                Err(error) => Signal::Invalid(format!("{error:#}")),
                            };
                            // Drop encoded bytes before delivering the parsed representation.
                            drop(bytes);
                            if state
                                .messages
                                .send_blocking(Delivery {
                                    signal,
                                    _reservation: Some(reservation),
                                })
                                .is_err()
                            {
                                return;
                            }
                        }
                        Ok(None) => {
                            if state.closed.load(Ordering::Acquire) {
                                return;
                            }
                            // Keep the normal exit ordered after all preceding messages.
                            thread::sleep(Duration::from_millis(50));
                            let tail: Vec<String> = log.lock().unwrap().iter().cloned().collect();
                            let mut text = "процесс расширения завершился".to_string();
                            if !tail.is_empty() {
                                text.push_str(":\n");
                                text.push_str(&tail.join("\n"));
                            }
                            let _ = state.messages.send_blocking(terminal(text));
                            state.messages.close();
                            return;
                        }
                        Err(error) => {
                            if !state.closed.load(Ordering::Acquire) {
                                state.stop(Some(format!("ошибка чтения расширения: {error:#}")));
                            }
                            return;
                        }
                    }
                }
            });
        }
        {
            let state = state.clone();
            thread::spawn(move || {
                while let Ok(frame) = lines.recv_blocking() {
                    if state.closed.load(Ordering::Acquire) {
                        return;
                    }
                    if let Err(error) = stdin.write_all(&frame.bytes).and_then(|()| stdin.flush()) {
                        state.stop(Some(format!("ошибка записи расширению: {error}")));
                        return;
                    }
                    // The reservation is released only AFTER the complete write/flush.
                }
            });
        }
        Ok((
            Self {
                state: state.clone(),
            },
            Signals {
                incoming,
                failure: errors,
                state,
            },
        ))
    }

    /// UI-safe admission. An error is terminal: accepted actions are never
    /// silently dropped or replayed, and the runtime receives Exited even when
    /// stdout was full. A successful admission does not promise a remote ACK.
    pub fn send(&self, message: &Outgoing) -> Result<()> {
        let result = self.enqueue(message);
        if let Err(error) = &result {
            self.state.stop(Some(format!(
                "не удалось отправить сообщение расширению: {error:#}"
            )));
        }
        result
    }

    fn enqueue(&self, message: &Outgoing) -> Result<()> {
        // Admit the producer count before even scanning the source. The first
        // serialization only counts UTF-8/escaping, allocating no JSON buffer.
        let mut reservation = self.state.outgoing.try_reserve(0, &self.state.closed)?;
        let size = io::encoded_size(message)?;
        reservation.try_grow(size, &self.state.closed)?;
        let mut bytes = Vec::with_capacity(size);
        serde_json::to_writer(&mut bytes, message)
            .context("не удалось записать JSON расширения")?;
        bytes.push(b'\n');
        debug_assert_eq!(bytes.len(), size);
        self.state
            .writer
            .try_send(Frame {
                bytes,
                _reservation: reservation,
            })
            .map_err(|_| anyhow::anyhow!("транспорт расширения закрыт или очередь заполнена"))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.state.stop(None);
    }
}
