//! Existing Alacritty implementation, retained for engine A/B.

use super::alacritty_screen as screen;
use super::{Base, Grid, Screen, Signal, Spawned, paste_bytes};
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use anyhow::{Context as _, Result};

#[derive(Clone)]
struct Listener {
    signals: async_channel::Sender<Signal>,
    redraw: async_channel::Sender<()>,
    pty: Arc<OnceLock<EventLoopSender>>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let signal = match event {
            // Coalesced: one pending redraw is enough however much arrives.
            Event::Wakeup => {
                let _ = self.redraw.try_send(());
                return;
            }
            Event::PtyWrite(text) => {
                if let Some(pty) = self.pty.get() {
                    let _ = pty.send(Msg::Input(Cow::Owned(text.into_bytes())));
                }
                return;
            }
            Event::Title(title) => Signal::Title(Some(title)),
            Event::ResetTitle => Signal::Title(None),
            Event::Bell => Signal::Bell,
            Event::Exit | Event::ChildExit(_) => Signal::Exited,
            _ => return,
        };
        let _ = self.signals.try_send(signal);
    }
}

impl Grid {
    fn window(self) -> WindowSize {
        WindowSize {
            num_lines: self.rows.min(u16::MAX as usize) as u16,
            num_cols: self.columns.min(u16::MAX as usize) as u16,
            cell_width: 1,
            cell_height: 1,
        }
    }
}

impl Dimensions for Grid {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    pty: EventLoopSender,
    grid: Grid,
}

impl Terminal {
    /// Starts the user's login shell (`$SHELL`) in `cwd`.
    pub fn spawn(cwd: &Path, grid: Grid) -> Result<Spawned> {
        let (signal_tx, signals) = async_channel::unbounded();
        let (redraw_tx, redraw) = async_channel::bounded(1);
        let shared = Arc::new(OnceLock::new());
        let listener = Listener {
            signals: signal_tx,
            redraw: redraw_tx,
            pty: shared.clone(),
        };
        let config = Config {
            scrolling_history: 10_000,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(config, &grid, listener.clone())));
        let options = tty::Options {
            shell: None,
            working_directory: Some(cwd.to_path_buf()),
            drain_on_exit: false,
            env: HashMap::from([
                ("TERM".to_owned(), "xterm-256color".to_owned()),
                ("COLORTERM".to_owned(), "truecolor".to_owned()),
            ]),
        };
        let pty = tty::new(&options, grid.window(), 0).context("не удалось открыть PTY")?;
        let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)
            .context("не удалось запустить терминал")?;
        let sender = event_loop.channel();
        let _ = shared.set(sender.clone());
        event_loop.spawn();
        Ok(Spawned {
            terminal: super::Terminal::Alacritty(Self {
                term,
                pty: sender,
                grid,
            }),
            signals,
            redraw,
        })
    }

    /// Sends keyboard input; typing always returns to the live screen.
    pub fn write(&self, bytes: Vec<u8>) {
        self.term.lock().scroll_display(Scroll::Bottom);
        let _ = self.pty.send(Msg::Input(Cow::Owned(bytes)));
    }

    pub fn paste(&self, text: &str) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        self.write(paste_bytes(text, bracketed));
    }

    pub fn app_cursor(&self) -> bool {
        self.mode().contains(TermMode::APP_CURSOR)
    }

    fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    pub fn resize(&mut self, grid: Grid) {
        if grid == self.grid {
            return;
        }
        self.grid = grid;
        self.term.lock().resize(grid);
        let _ = self.pty.send(Msg::Resize(grid.window()));
    }

    /// Scrolls the history; positive moves up.
    pub fn scroll(&self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn snapshot(&self, base: Base) -> Screen {
        screen::snapshot(&self.term.lock(), base)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.pty.send(Msg::Shutdown);
    }
}
