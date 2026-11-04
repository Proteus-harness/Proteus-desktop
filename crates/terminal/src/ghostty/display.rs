//! Ghostty parser/render state for a remote byte stream, with no PTY ownership.
use crate::{Base, Grid, Key, Screen, Signal};
use anyhow::Result;
use ghostty_vt::terminal::{CompressionMode, CompressionResult, Event, Mode, ScrollViewport};
use ghostty_vt::{RenderState, Terminal};
use std::time::{Duration, Instant};

pub(crate) struct Engine {
    vt: Terminal,
    render: RenderState,
    rows: ghostty_vt::render::RowIterator,
    cells: ghostty_vt::render::CellIterator,
    encoder: ghostty_vt::key::Encoder,
    key_event: ghostty_vt::key::Event,
    activity: ghostty_vt::terminal::CompressionActivity,
    compression: Option<Instant>,
    hold: Option<Instant>,
}
impl Engine {
    pub(crate) fn new(grid: Grid, base: Base) -> Result<Self> {
        let vt = super::configured_vt(grid, base)?;
        let activity = vt.compression_activity()?;
        Ok(Self {
            vt,
            render: RenderState::new()?,
            rows: ghostty_vt::render::RowIterator::new()?,
            cells: ghostty_vt::render::CellIterator::new()?,
            encoder: ghostty_vt::key::Encoder::new()?,
            key_event: ghostty_vt::key::Event::new()?,
            activity,
            compression: None,
            hold: None,
        })
    }
    pub(crate) fn feed(&mut self, bytes: &[u8], signals: &async_channel::Sender<Signal>) {
        self.vt.vt_write(bytes);
        for event in self.vt.events() {
            let signal = match event {
                Event::TitleChanged(title) => Some(Signal::Title(Some(title))),
                Event::Bell => Some(Signal::Bell),
                Event::RenderHold(value) => {
                    self.hold = value.then(Instant::now);
                    None
                }
                // No automatic replies: multiple viewers must not write into a process.
                Event::PtyWrite(_) => None,
                _ => None,
            };
            if let Some(signal) = signal {
                let _ = signals.try_send(signal);
            }
        }
    }
    pub(crate) fn key(&mut self, key: Key<'_>) -> Result<Vec<u8>> {
        match super::input::KeyInput::new(key) {
            Some(key) => key.encode(&self.vt, &mut self.encoder, &mut self.key_event),
            None => Ok(Vec::new()),
        }
    }
    pub(crate) fn bracketed_paste(&self) -> bool {
        self.vt.mode(Mode::BRACKETED_PASTE).unwrap_or(false)
    }
    pub(crate) fn resize(&mut self, grid: Grid) -> Result<()> {
        self.vt
            .resize(grid.columns as u16, grid.rows as u16, 1, 1)?;
        Ok(())
    }
    pub(crate) fn set_base(&mut self, base: Base) -> Result<()> {
        self.vt
            .set_default_fg_color(Some(super::rgb(base.foreground)))?;
        self.vt
            .set_default_bg_color(Some(super::rgb(base.background)))?;
        Ok(())
    }
    pub(crate) fn scroll(&mut self, lines: Option<i32>) {
        self.vt
            .scroll_viewport(lines.map_or(ScrollViewport::Bottom, |lines| {
                ScrollViewport::Delta(-(lines as isize))
            }));
    }
    pub(crate) fn snapshot(&mut self, base: Base) -> Result<Screen> {
        super::snapshot::snapshot(
            &self.vt,
            &mut self.render,
            &mut self.rows,
            &mut self.cells,
            base,
        )
    }
    pub(crate) fn maintain(&mut self) -> Result<()> {
        let activity = self.vt.compression_activity()?;
        if activity != self.activity {
            self.activity = activity;
            self.compression = Some(Instant::now() + Duration::from_millis(250));
        }
        if self.compression.is_some_and(|due| due <= Instant::now()) {
            self.compression = matches!(
                self.vt.compress(CompressionMode::Incremental)?,
                CompressionResult::Pending
            )
            .then(|| Instant::now() + super::FRAME_TIME);
        }
        Ok(())
    }
    pub(crate) fn maintenance_due(&self) -> Option<Instant> {
        self.compression
    }
    pub(crate) fn render_allowed(&self) -> bool {
        self.hold
            .is_none_or(|since| since.elapsed() > Duration::from_millis(250))
    }
}
