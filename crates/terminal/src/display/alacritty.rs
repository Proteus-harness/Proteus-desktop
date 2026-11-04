//! Zed's display-only Term/Processor pattern, without tty or EventLoop.
use crate::{Base, Grid, Screen, Signal};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

#[derive(Clone)]
struct Listener(async_channel::Sender<Signal>);
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let signal = match event {
            Event::Title(title) => Signal::Title(Some(title)),
            Event::ResetTitle => Signal::Title(None),
            Event::Bell => Signal::Bell,
            // In particular ignore PtyWrite: observers do not answer queries.
            _ => return,
        };
        let _ = self.0.try_send(signal);
    }
}
pub(super) struct Engine {
    term: Term<Listener>,
    parser: Processor,
}
fn config(grid: Grid) -> Config {
    let line_bytes = grid
        .columns
        .saturating_mul(std::mem::size_of::<alacritty_terminal::term::cell::Cell>())
        .max(1);
    Config {
        scrolling_history: (8 * 1024 * 1024 / line_bytes).min(10_000),
        ..Config::default()
    }
}
impl Engine {
    pub(super) fn new(grid: Grid, signals: async_channel::Sender<Signal>) -> Self {
        Self {
            term: Term::new(config(grid), &grid, Listener(signals)),
            parser: Processor::new(),
        }
    }
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }
    pub(super) fn app_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }
    pub(super) fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }
    pub(super) fn resize(&mut self, grid: Grid) {
        self.term.set_options(config(grid));
        self.term.resize(grid);
    }
    pub(super) fn scroll(&mut self, lines: Option<i32>) {
        self.term
            .scroll_display(lines.map_or(Scroll::Bottom, Scroll::Delta));
    }
    pub(super) fn snapshot(&self, base: Base) -> Screen {
        crate::alacritty_screen::snapshot(&self.term, base)
    }
}
