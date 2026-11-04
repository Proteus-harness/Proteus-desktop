//! VT for an existing remote process: bytes in, encoded user input out.
//! Adapted from Zed crates/terminal/src/terminal.rs::new_display_only.
//! This actor owns no shell, PTY, process handle or automatic query replies.
mod alacritty;
mod engine;
#[cfg(test)]
mod tests;

use crate::{Base, Grid, Key, Screen, Signal};
use anyhow::{Context as _, Result};
use engine::Engine;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const QUEUE_BYTES: usize = 1024 * 1024;
const COMMANDS: usize = 64;
const INPUT_CHUNK: usize = 16 * 1024;
const FRAME_TIME: Duration = Duration::from_millis(16);
const BASE: Base = Base {
    foreground: [200; 3],
    background: [0; 3],
};

pub struct SpawnedDisplay {
    pub display: Display,
    pub signals: async_channel::Receiver<Signal>,
    pub redraw: async_channel::Receiver<()>,
    /// Encoded user key/paste bytes only; output never echoes input locally.
    pub input: async_channel::Receiver<Vec<u8>>,
}

struct View {
    screen: Screen,
    base: Base,
    grid: Grid,
    scroll: i32,
}
struct Shared {
    view: Mutex<View>,
    output_bytes: AtomicUsize,
    input_bytes: AtomicUsize,
    frame_credit: AtomicBool,
    shutdown: AtomicBool,
}

struct OwnedKey {
    name: String,
    text: Option<String>,
    ctrl: bool,
    alt: bool,
    shift: bool,
}
impl OwnedKey {
    fn new(key: Key<'_>) -> Option<Self> {
        if key.key.len() > 64 || key.text.is_some_and(|text| text.len() > 4096) {
            return None;
        }
        crate::encode(key, false)?;
        Some(Self {
            name: key.key.into(),
            text: key.text.map(str::to_owned),
            ctrl: key.ctrl,
            alt: key.alt,
            shift: key.shift,
        })
    }
    fn borrowed(&self) -> Key<'_> {
        Key {
            key: &self.name,
            text: self.text.as_deref(),
            ctrl: self.ctrl,
            alt: self.alt,
            shift: self.shift,
        }
    }
    fn reserved(&self) -> usize {
        self.text.as_ref().map_or(0, String::len) + 128
    }
}
enum Command {
    Feed(Vec<u8>),
    Key(OwnedKey),
    Paste(String),
    Reset,
    Wake,
}
impl Command {
    fn reservation(&self) -> (usize, bool) {
        match self {
            Self::Feed(bytes) => (bytes.len(), false),
            Self::Key(key) => (key.reserved(), true),
            Self::Paste(text) => (text.len() + 12, true),
            _ => (0, false),
        }
    }
}

pub struct Display {
    shared: Arc<Shared>,
    commands: mpsc::SyncSender<Command>,
    signals: async_channel::Sender<Signal>,
}
impl Display {
    pub fn new(grid: Grid) -> Result<SpawnedDisplay> {
        Self::with_engine(grid, &crate::selected_engine())
    }
    fn with_engine(grid: Grid, engine: &str) -> Result<SpawnedDisplay> {
        let grid = Grid::new(grid.columns, grid.rows);
        let shared = Arc::new(Shared {
            view: Mutex::new(View {
                screen: Screen {
                    columns: grid.columns,
                    rows: vec![vec![]; grid.rows],
                    cursor: None,
                    scrolled: 0,
                },
                base: BASE,
                grid,
                scroll: 0,
            }),
            output_bytes: AtomicUsize::new(0),
            input_bytes: AtomicUsize::new(0),
            frame_credit: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
        });
        let (commands, incoming) = mpsc::sync_channel(COMMANDS);
        let (signals, signal_rx) = async_channel::bounded(16);
        let (redraw, redraw_rx) = async_channel::bounded(1);
        // Sole producer emits <=16 KiB chunks, so this queue retains <=1 MiB.
        let (input, input_rx) = async_channel::bounded(COMMANDS);
        let (ready, started) = mpsc::sync_channel(1);
        let owned = shared.clone();
        let errors = signals.clone();
        let name = engine.to_owned();
        std::thread::Builder::new()
            .name("terminal-display".into())
            .spawn(move || {
                let result = run(&name, &owned, incoming, &errors, redraw, input, ready);
                if let Err(error) = result {
                    let _ = errors.try_send(Signal::Failed(format!("Терминал: {error:#}")));
                }
            })
            .context("не удалось запустить отображение терминала")?;
        let initialized = started.recv_timeout(Duration::from_secs(10));
        if !matches!(initialized, Ok(Ok(()))) {
            shared.shutdown.store(true, Ordering::Release);
            let _ = commands.try_send(Command::Wake);
            initialized
                .context("отображение терминала не запустилось")?
                .map_err(anyhow::Error::msg)?;
        }
        Ok(SpawnedDisplay {
            display: Self {
                shared,
                commands,
                signals,
            },
            signals: signal_rx,
            redraw: redraw_rx,
            input: input_rx,
        })
    }
    /// False leaves ownership of the remote cursor with the caller: retry later.
    pub fn feed(&self, bytes: Vec<u8>) -> bool {
        self.try_feed(bytes).is_ok()
    }
    /// Like feed, returning the same allocation when backpressure rejects it.
    pub fn try_feed(&self, bytes: Vec<u8>) -> std::result::Result<(), Vec<u8>> {
        if self.shared.shutdown.load(Ordering::Acquire) {
            return Err(bytes);
        }
        let reserved = bytes.len();
        let counter = &self.shared.output_bytes;
        if counter
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |old| {
                old.checked_add(reserved)
                    .filter(|total| *total <= QUEUE_BYTES)
            })
            .is_err()
        {
            return Err(bytes);
        }
        match self.commands.try_send(Command::Feed(bytes)) {
            Ok(()) => Ok(()),
            Err(
                mpsc::TrySendError::Full(Command::Feed(bytes))
                | mpsc::TrySendError::Disconnected(Command::Feed(bytes)),
            ) => {
                counter.fetch_sub(reserved, Ordering::AcqRel);
                Err(bytes)
            }
            Err(_) => unreachable!("only Feed was sent"),
        }
    }
    /// Ordered with feed; clears parser state and scrollback after a remote gap.
    pub fn reset(&self) -> bool {
        self.enqueue(Command::Reset)
    }
    pub fn key(&self, key: Key<'_>) -> bool {
        let Some(key) = OwnedKey::new(key) else {
            return false;
        };
        self.enqueue(Command::Key(key));
        true
    }
    pub fn paste(&self, text: &str) {
        if text.len() > QUEUE_BYTES - 12 {
            self.reject();
            return;
        }
        self.enqueue_with(text.len() + 12, true, || Command::Paste(text.into()));
    }
    fn reject(&self) {
        let _ = self.signals.try_send(Signal::InputRejected(
            "Очередь ввода терминала заполнена".into(),
        ));
    }
    fn enqueue(&self, command: Command) -> bool {
        let (bytes, input) = command.reservation();
        self.enqueue_with(bytes, input, || command)
    }
    fn enqueue_with(&self, bytes: usize, input: bool, command: impl FnOnce() -> Command) -> bool {
        if self.shared.shutdown.load(Ordering::Acquire) {
            return false;
        }
        let counter = if input {
            &self.shared.input_bytes
        } else {
            &self.shared.output_bytes
        };
        if counter
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |old| {
                old.checked_add(bytes).filter(|total| *total <= QUEUE_BYTES)
            })
            .is_err()
        {
            if input {
                self.reject();
            }
            return false;
        }
        if self.commands.try_send(command()).is_err() {
            counter.fetch_sub(bytes, Ordering::AcqRel);
            if input {
                self.reject();
            }
            return false;
        }
        true
    }
    fn wake(&self) {
        let _ = self.commands.try_send(Command::Wake);
    }
    pub fn resize(&mut self, grid: Grid) {
        self.shared.view.lock().unwrap().grid = Grid::new(grid.columns, grid.rows);
        self.wake();
    }
    pub fn scroll(&self, lines: i32) {
        let mut view = self.shared.view.lock().unwrap();
        view.scroll = view.scroll.saturating_add(lines);
        drop(view);
        self.shared.frame_credit.store(true, Ordering::Release);
        self.wake();
    }
    pub fn snapshot(&self, base: Base) -> Screen {
        let mut view = self.shared.view.lock().unwrap();
        view.base = base;
        let screen = view.screen.clone();
        drop(view);
        self.shared.frame_credit.store(true, Ordering::Release);
        self.wake();
        screen
    }
}
impl Drop for Display {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.wake();
    }
}

fn run(
    name: &str,
    shared: &Shared,
    incoming: mpsc::Receiver<Command>,
    signals: &async_channel::Sender<Signal>,
    redraw: async_channel::Sender<()>,
    input: async_channel::Sender<Vec<u8>>,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<()> {
    let mut grid = shared.view.lock().unwrap().grid;
    let mut engine = match Engine::new(name, grid, BASE, signals.clone()) {
        Ok(engine) => {
            let _ = ready.send(Ok(()));
            engine
        }
        Err(error) => {
            let _ = ready.send(Err(format!("{error:#}")));
            return Err(error);
        }
    };
    let mut base = BASE;
    let mut dirty = true;
    let mut last_frame = Instant::now() - FRAME_TIME;
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            break;
        }
        let (wanted_grid, wanted_base, scroll) = {
            let mut view = shared.view.lock().unwrap();
            (view.grid, view.base, std::mem::take(&mut view.scroll))
        };
        if grid != wanted_grid {
            grid = wanted_grid;
            engine.resize(grid)?;
            dirty = true;
        }
        if base != wanted_base {
            base = wanted_base;
            engine.set_base(base)?;
            dirty = true;
        }
        if scroll != 0 {
            engine.scroll(Some(scroll));
            dirty = true;
        }
        engine.maintain()?;
        if dirty
            && engine.render_allowed()
            && last_frame.elapsed() >= FRAME_TIME
            && shared.frame_credit.swap(false, Ordering::AcqRel)
        {
            let screen = engine.snapshot(base)?;
            shared.view.lock().unwrap().screen = screen;
            let _ = redraw.try_send(());
            last_frame = Instant::now();
            dirty = false;
        }
        let frame_due = (dirty && shared.frame_credit.load(Ordering::Acquire)).then(|| {
            if engine.render_allowed() {
                last_frame + FRAME_TIME
            } else {
                Instant::now() + Duration::from_millis(50)
            }
        });
        let due = frame_due.into_iter().chain(engine.maintenance_due()).min();
        let command = match due {
            Some(due) => match incoming.recv_timeout(due.saturating_duration_since(Instant::now()))
            {
                Ok(command) => command,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            },
            None => match incoming.recv() {
                Ok(command) => command,
                Err(_) => break,
            },
        };
        let (reserved, user) = command.reservation();
        match command {
            Command::Feed(bytes) => {
                engine.feed(&bytes, signals)?;
                dirty = true;
            }
            Command::Reset => {
                engine = Engine::new(name, grid, base, signals.clone())?;
                dirty = true;
            }
            Command::Key(key) => {
                let bytes = engine.key(key.borrowed())?;
                deliver_input(bytes, &input, signals);
                engine.scroll(None);
                dirty = true;
            }
            Command::Paste(text) => {
                let bytes = crate::paste_bytes(&text, engine.bracketed_paste());
                deliver_input(bytes, &input, signals);
                engine.scroll(None);
                dirty = true;
            }
            Command::Wake => {}
        }
        let counter = if user {
            &shared.input_bytes
        } else {
            &shared.output_bytes
        };
        counter.fetch_sub(reserved, Ordering::AcqRel);
    }
    Ok(())
}

fn deliver_input(
    bytes: Vec<u8>,
    input: &async_channel::Sender<Vec<u8>>,
    signals: &async_channel::Sender<Signal>,
) {
    if bytes.is_empty() {
        return;
    }
    let needed = bytes.len().div_ceil(INPUT_CHUNK);
    if needed > COMMANDS.saturating_sub(input.len()) || input.is_closed() {
        let _ = signals.try_send(Signal::InputRejected(
            "Очередь ввода терминала заполнена".into(),
        ));
        return;
    }
    if needed == 1 {
        let _ = input.try_send(bytes);
        return;
    }
    for chunk in bytes.chunks(INPUT_CHUNK) {
        if input.try_send(chunk.to_vec()).is_err() {
            break;
        }
    }
}
