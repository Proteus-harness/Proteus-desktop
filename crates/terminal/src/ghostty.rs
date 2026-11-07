//! Ghostty owns parsing on one thread; the UI only clones a visible frame.
//!
//! PTY descriptors, process lifecycle and resize use portable-pty. There is
//! no Alacritty emulator or raw-output replay buffer in this implementation.

pub(super) mod display;
mod input;
mod process;
use process::Process;
mod snapshot;

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use ghostty_vt::terminal::{
    ClipboardWritePolicy, CompressionMode, CompressionResult, Event, Mode, Options, ScrollViewport,
};
use ghostty_vt::{RenderState, Terminal as Vt};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use super::{Base, Grid, Screen, Signal, Spawned, paste_bytes};

const INPUT_LIMIT: usize = 1024 * 1024;
const READ_CHUNK: usize = 16 * 1024;
const FRAME_TIME: Duration = Duration::from_millis(16);
const BASE: Base = Base {
    foreground: [200; 3],
    background: [0; 3],
};

struct View {
    screen: Screen,
    base: Base,
    grid: Grid,
    scroll: i32,
    bottom: bool,
}

struct Shared {
    view: Mutex<View>,
    modes: AtomicU8,
    input_bytes: AtomicUsize,
    frame_credit: AtomicBool,
    shutdown: AtomicBool,
    exited: AtomicBool,
    done: AtomicBool,
    wake: UnixStream,
}

enum Input {
    Bytes(Vec<u8>),
    Key(input::KeyInput),
}
impl Input {
    fn reserved(&self) -> usize {
        match self {
            Self::Bytes(bytes) => bytes.len(),
            Self::Key(key) => key.reserved(),
        }
    }
}

pub struct Terminal {
    shared: Arc<Shared>,
    input: mpsc::SyncSender<Input>,
    signals: async_channel::Sender<Signal>,
}

impl Terminal {
    pub fn spawn(cwd: &Path, grid: Grid) -> Result<Spawned> {
        Self::spawn_shell(cwd, grid, None)
    }

    fn spawn_shell(cwd: &Path, grid: Grid, shell: Option<&str>) -> Result<Spawned> {
        let pair = native_pty_system()
            .openpty(size(grid))
            .context("не удалось открыть PTY")?;
        let shell = shell.map(str::to_owned).unwrap_or_else(|| {
            std::env::var("SHELL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/bin/sh".into())
        });
        let mut command = CommandBuilder::new(shell);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .context("не удалось запустить shell")?;
        drop(pair.slave);
        let process = Process::new(pair.master, child)?;
        let (wake, receiver) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        receiver.set_nonblocking(true)?;
        let (input, incoming) = mpsc::sync_channel(64);
        let (signal_tx, signals) = async_channel::bounded(16);
        let (redraw_tx, redraw) = async_channel::bounded(1);
        let shared = Arc::new(Shared {
            view: Mutex::new(View {
                screen: Screen {
                    columns: grid.columns,
                    rows: vec![Vec::new(); grid.rows],
                    cursor: None,
                    scrolled: 0,
                },
                base: BASE,
                grid,
                scroll: 0,
                bottom: false,
            }),
            modes: AtomicU8::new(0),
            input_bytes: AtomicUsize::new(0),
            frame_credit: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            done: AtomicBool::new(false),
            wake,
        });
        register(&shared);
        let owned = shared.clone();
        let errors = signal_tx.clone();
        let (ready, started) = mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("ghostty-pty".into())
            .spawn(move || {
                let result = run(
                    process, &owned, incoming, receiver, &errors, &redraw_tx, ready,
                );
                owned.done.store(true, Ordering::Release);
                if let Err(error) = result {
                    let _ = errors.try_send(Signal::Failed(format!("Ghostty: {error:#}")));
                }
                if !owned.exited.swap(true, Ordering::AcqRel) {
                    let _ = errors.send_blocking(Signal::Exited);
                }
            })
            .context("не удалось запустить Ghostty")?;
        let initialization = started.recv_timeout(Duration::from_secs(10));
        if !matches!(initialization, Ok(Ok(()))) {
            shared.shutdown.store(true, Ordering::Release);
            let _ = (&shared.wake).write(&[1]);
            initialization
                .context("Ghostty не запустился")?
                .map_err(anyhow::Error::msg)?;
        }
        Ok(Spawned {
            terminal: super::Terminal::Ghostty(Self {
                shared,
                input,
                signals: signal_tx,
            }),
            signals,
            redraw,
        })
    }

    fn wake(&self) {
        let _ = (&self.shared.wake).write(&[1]);
    }

    pub fn key(&self, key: super::Key<'_>) -> bool {
        if self.shared.exited.load(Ordering::Acquire) {
            return false;
        }
        let Some(key) = input::KeyInput::new(key) else {
            return false;
        };
        self.enqueue(Input::Key(key));
        true
    }

    pub fn write(&self, bytes: Vec<u8>) {
        self.enqueue(Input::Bytes(bytes));
    }

    fn enqueue(&self, input: Input) {
        if self.shared.exited.load(Ordering::Acquire) {
            return;
        }
        let len = input.reserved();
        if len == 0 {
            return;
        }
        let reserved = self
            .shared
            .input_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Relaxed, |old| {
                old.checked_add(len).filter(|&n| n <= INPUT_LIMIT)
            })
            .is_ok();
        if !reserved {
            let _ = self.signals.try_send(Signal::InputRejected(
                "Ввод терминала превышает очередь 1 МиБ".into(),
            ));
            return;
        }
        if self.input.try_send(input).is_err() {
            self.shared.input_bytes.fetch_sub(len, Ordering::AcqRel);
            let _ = self.signals.try_send(Signal::InputRejected(
                "Очередь ввода терминала заполнена".into(),
            ));
            return;
        }
        self.shared.view.lock().unwrap().bottom = true;
        self.wake();
    }

    pub fn paste(&self, text: &str) {
        if text.len() > INPUT_LIMIT - 12 {
            let _ = self.signals.try_send(Signal::InputRejected(
                "Вставка терминала превышает 1 МиБ".into(),
            ));
            return;
        }
        self.write(paste_bytes(
            text,
            self.shared.modes.load(Ordering::Acquire) & 2 != 0,
        ));
    }
    pub fn app_cursor(&self) -> bool {
        self.shared.modes.load(Ordering::Acquire) & 1 != 0
    }
    pub fn resize(&mut self, grid: Grid) {
        let mut view = self.shared.view.lock().unwrap();
        if view.grid == grid {
            return;
        }
        view.grid = grid;
        drop(view);
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

impl Drop for Terminal {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Release);
        self.wake();
    }
}

fn size(grid: Grid) -> PtySize {
    PtySize {
        rows: grid.rows as u16,
        cols: grid.columns as u16,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn run(
    process: Process,
    shared: &Shared,
    incoming: mpsc::Receiver<Input>,
    mut wake: UnixStream,
    signals: &async_channel::Sender<Signal>,
    redraw: &async_channel::Sender<()>,
    ready: mpsc::SyncSender<Result<(), String>>,
) -> Result<()> {
    let init = || -> Result<_> {
        let grid = shared.view.lock().unwrap().grid;
        let vt = configured_vt(grid, BASE)?;
        let fd = process
            .master
            .as_raw_fd()
            .context("PTY не предоставляет fd")?;
        // SAFETY: this actor owns the master until it observes PTY EOF.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error().into());
        }
        Ok((vt, RenderState::new()?, fd, grid))
    };
    let (mut vt, mut render, fd, mut grid) = match init() {
        Ok(value) => {
            let _ = ready.send(Ok(()));
            value
        }
        Err(error) => {
            let _ = ready.send(Err(format!("{error:#}")));
            return Err(error);
        }
    };
    let mut process = Some(process);
    let mut queue = VecDeque::<(Vec<u8>, usize, usize)>::new();
    let mut encoder = ghostty_vt::key::Encoder::new()?;
    let mut key_event = ghostty_vt::key::Event::new()?;
    let mut reply_bytes = 0usize;
    let mut read = [0u8; READ_CHUNK];
    let mut dirty = true;
    let mut base = BASE;
    let mut held = false;
    let mut held_since = Instant::now();
    let mut last_frame = Instant::now() - FRAME_TIME;
    let mut compression_activity = vt.compression_activity()?;
    let mut compression_due: Option<Instant> = None;
    let mut rows = ghostty_vt::render::RowIterator::new()?;
    let mut cells = ghostty_vt::render::CellIterator::new()?;
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            break;
        }
        while let Ok(input) = incoming.try_recv() {
            let reserved = input.reserved();
            if process.is_none() {
                shared.input_bytes.fetch_sub(reserved, Ordering::AcqRel);
                continue;
            }
            let bytes = match input {
                Input::Bytes(bytes) => bytes,
                Input::Key(key) => key.encode(&vt, &mut encoder, &mut key_event)?,
            };
            if bytes.is_empty() {
                shared.input_bytes.fetch_sub(reserved, Ordering::AcqRel);
                continue;
            }
            if bytes.len() > reserved {
                bail!("ответ кодировщика клавиш превышает резерв ввода");
            }
            queue.push_back((bytes, 0, reserved));
        }
        let (wanted_grid, wanted_base, bottom, scroll) = {
            let mut view = shared.view.lock().unwrap();
            (
                view.grid,
                view.base,
                std::mem::take(&mut view.bottom),
                std::mem::take(&mut view.scroll),
            )
        };
        if grid != wanted_grid {
            grid = wanted_grid;
            vt.resize(grid.columns as u16, grid.rows as u16, 1, 1)?;
            if let Some(process) = &process {
                if let Err(error) = process.master.resize(size(grid)) {
                    // A slave can close between a size request and its final
                    // readable bytes. Let the read path drain that EOF.
                    if error
                        .downcast_ref::<io::Error>()
                        .and_then(io::Error::raw_os_error)
                        != Some(libc::EIO)
                    {
                        return Err(error);
                    }
                }
            }
            dirty = true;
        }
        if base != wanted_base {
            base = wanted_base;
            vt.set_default_fg_color(Some(rgb(base.foreground)))?;
            vt.set_default_bg_color(Some(rgb(base.background)))?;
            dirty = true;
        }
        if bottom {
            vt.scroll_viewport(ScrollViewport::Bottom);
            dirty = true;
        }
        if scroll != 0 {
            vt.scroll_viewport(ScrollViewport::Delta(-(scroll as isize)));
            dirty = true;
        }
        if process.is_some() {
            effects(
                &mut vt,
                signals,
                &mut queue,
                &mut reply_bytes,
                &mut held,
                &mut held_since,
            )?;
        } else {
            // Retained rendering cannot send terminal replies to a closed PTY.
            vt.events().for_each(drop);
        }
        // A bounded write burst gives output a chance to progress during pastes.
        let mut written = 0usize;
        while let Some((bytes, offset, reserved)) = queue.front_mut() {
            let len = (bytes.len() - *offset).min(8192);
            // SAFETY: borrowed bytes and owned fd remain valid for the call.
            let n = unsafe { libc::write(fd, bytes[*offset..].as_ptr().cast(), len) };
            if n < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if matches!(error.raw_os_error(), Some(libc::EIO | libc::EPIPE)) {
                    // Pending input must not hide output already waiting when
                    // the slave closes. The read loop owns EOF and cleanup.
                    break;
                }
                return Err(error.into());
            }
            if n == 0 {
                break;
            }
            *offset += n as usize;
            written += n as usize;
            if *offset == bytes.len() {
                if *reserved != 0 {
                    shared.input_bytes.fetch_sub(*reserved, Ordering::AcqRel);
                } else {
                    reply_bytes -= bytes.len();
                }
                queue.pop_front();
            }
            if written >= 64 * 1024 {
                break;
            }
        }
        let mut eof = false;
        // Avoid a producer keeping this loop forever while resize/input waits.
        for _ in 0..if process.is_some() { 4 } else { 0 } {
            let n = unsafe { libc::read(fd, read.as_mut_ptr().cast(), read.len()) };
            if n == 0 {
                eof = true;
                break;
            }
            if n < 0 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::WouldBlock {
                    break;
                }
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                if error.raw_os_error() == Some(libc::EIO) {
                    eof = true;
                    break;
                }
                return Err(error.into());
            }
            vt.vt_write(&read[..n as usize]);
            dirty = true;
            effects(
                &mut vt,
                signals,
                &mut queue,
                &mut reply_bytes,
                &mut held,
                &mut held_since,
            )?;
        }
        shared.modes.store(
            u8::from(vt.mode(Mode::DECCKM)?) | (u8::from(vt.mode(Mode::BRACKETED_PASTE)?) << 1),
            Ordering::Release,
        );
        let allow_frame = !held || held_since.elapsed() > Duration::from_millis(250);
        // EOF is a terminal boundary, not a replaceable animation frame: publish
        // all parsed bytes even when the UI has spent its credit or output is held.
        if (dirty || eof)
            && (eof
                || (allow_frame
                    && last_frame.elapsed() >= FRAME_TIME
                    && shared.frame_credit.swap(false, Ordering::AcqRel)))
        {
            let mut screen = snapshot::snapshot(&vt, &mut render, &mut rows, &mut cells, base)?;
            if eof || process.is_none() {
                screen.cursor = None;
            }
            let old = std::mem::replace(&mut shared.view.lock().unwrap().screen, screen);
            drop(old);
            let _ = redraw.try_send(());
            last_frame = Instant::now();
            dirty = false;
        }
        if eof {
            shared.exited.store(true, Ordering::Release);
            for (_, _, reserved) in queue.drain(..) {
                shared.input_bytes.fetch_sub(reserved, Ordering::AcqRel);
            }
            reply_bytes = 0;
            // Reap and close the PTY now. The existing actor keeps only the
            // bounded VT for read-only scroll/resize/theme until its view drops.
            drop(process.take());
            held = false;
            let _ = signals.send_blocking(Signal::Exited);
        }
        let activity = vt.compression_activity()?;
        if activity != compression_activity {
            compression_activity = activity;
            compression_due = Some(Instant::now() + Duration::from_millis(250));
        }
        if compression_due.is_some_and(|due| due <= Instant::now()) {
            compression_due = matches!(
                vt.compress(CompressionMode::Incremental)?,
                CompressionResult::Pending
            )
            .then(|| Instant::now() + FRAME_TIME);
        }
        let timer = dirty && allow_frame && shared.frame_credit.load(Ordering::Acquire);
        let timeout = if timer {
            (FRAME_TIME.saturating_sub(last_frame.elapsed()).as_millis() as i32).max(1)
        } else if held {
            50
        } else {
            -1
        };
        let timeout = match compression_due {
            Some(due) => {
                let remaining = due
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .clamp(1, i32::MAX as u128) as i32;
                if timeout < 0 {
                    remaining
                } else {
                    timeout.min(remaining)
                }
            }
            None => timeout,
        };
        let mut fds = [
            libc::pollfd {
                fd: if process.is_some() { fd } else { -1 },
                events: libc::POLLIN | if queue.is_empty() { 0 } else { libc::POLLOUT },
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
        if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return Err(io::Error::last_os_error().into());
        }
        while wake.read(&mut read).is_ok_and(|n| n != 0) {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub(super) fn configured_vt(grid: Grid, base: Base) -> Result<Vt> {
    let mut vt = Vt::new(Options {
        cols: grid.columns as u16,
        rows: grid.rows as u16,
        max_scrollback: 8 * 1024 * 1024,
    })?;
    vt.resize(grid.columns as u16, grid.rows as u16, 1, 1)?;
    vt.set_default_fg_color(Some(rgb(base.foreground)))?;
    vt.set_default_bg_color(Some(rgb(base.background)))?;
    vt.set_default_color_palette(Some(ghostty_vt::style::Palette(std::array::from_fn(|i| {
        rgb(super::alacritty_screen::indexed(i as u8))
    }))))?;
    vt.set_scrollback_max_lines(Some(10_000))?;
    vt.set_apc_max_bytes(Some(64 * 1024))?;
    vt.set_clipboard_write_max_bytes(Some(0))?;
    vt.set_clipboard_write_policy(ClipboardWritePolicy::Deny);
    vt.set_glyph_protocol_enabled(false)?;
    vt.set_xtversion(Some("Proteus Ghostty".into()));
    Ok(vt)
}

fn rgb([r, g, b]: [u8; 3]) -> ghostty_vt::style::RgbColor {
    ghostty_vt::style::RgbColor { r, g, b }
}

fn effects(
    vt: &mut Vt,
    signals: &async_channel::Sender<Signal>,
    queue: &mut VecDeque<(Vec<u8>, usize, usize)>,
    reply_bytes: &mut usize,
    held: &mut bool,
    held_since: &mut Instant,
) -> Result<()> {
    for event in vt.events() {
        match event {
            Event::PtyWrite(bytes) => {
                *reply_bytes += bytes.len();
                if *reply_bytes > 64 * 1024 {
                    bail!("очередь ответов PTY превышает 64 КиБ");
                }
                queue.push_back((bytes, 0, 0));
            }
            Event::TitleChanged(title) => {
                let _ = signals.try_send(Signal::Title(Some(title)));
            }
            Event::Bell => {
                let _ = signals.try_send(Signal::Bell);
            }
            Event::RenderHold(value) => {
                *held = value;
                *held_since = Instant::now();
            }
            _ => {}
        }
    }
    Ok(())
}

fn actors() -> &'static Mutex<Vec<Weak<Shared>>> {
    static ACTORS: OnceLock<Mutex<Vec<Weak<Shared>>>> = OnceLock::new();
    ACTORS.get_or_init(Mutex::default)
}
fn register(shared: &Arc<Shared>) {
    let mut actors = actors().lock().unwrap();
    actors.retain(|actor| {
        actor
            .upgrade()
            .is_some_and(|actor| !actor.done.load(Ordering::Acquire))
    });
    actors.push(Arc::downgrade(shared));
}

pub(super) fn shutdown_all() {
    let active: Vec<_> = {
        let mut actors = actors().lock().unwrap();
        actors.retain(|actor| {
            actor
                .upgrade()
                .is_some_and(|actor| !actor.done.load(Ordering::Acquire))
        });
        actors.iter().filter_map(Weak::upgrade).collect()
    };
    for actor in &active {
        actor.shutdown.store(true, Ordering::Release);
        let _ = (&actor.wake).write(&[1]);
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    while active
        .iter()
        .any(|actor| !actor.done.load(Ordering::Acquire))
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(5));
    }
}
