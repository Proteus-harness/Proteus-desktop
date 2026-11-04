use crate::{Base, Grid, Key, Screen, Signal};
use anyhow::Result;
use std::time::Instant;

pub(super) enum Engine {
    Alacritty(super::alacritty::Engine),
    #[cfg(all(feature = "ghostty-vt", unix))]
    Ghostty(crate::ghostty::display::Engine),
}
impl Engine {
    pub(super) fn new(
        name: &str,
        grid: Grid,
        base: Base,
        signals: async_channel::Sender<Signal>,
    ) -> Result<Self> {
        match name {
            "alacritty" => Ok(Self::Alacritty(super::alacritty::Engine::new(
                grid, signals,
            ))),
            "ghostty" => {
                #[cfg(all(feature = "ghostty-vt", unix))]
                {
                    return Ok(Self::Ghostty(crate::ghostty::display::Engine::new(
                        grid, base,
                    )?));
                }
                #[cfg(not(all(feature = "ghostty-vt", unix)))]
                {
                    let _ = base;
                    anyhow::bail!("Эта сборка не содержит движок Ghostty");
                }
            }
            other => anyhow::bail!("Неизвестный движок терминала: {other}"),
        }
    }
    pub(super) fn feed(
        &mut self,
        bytes: &[u8],
        signals: &async_channel::Sender<Signal>,
    ) -> Result<()> {
        #[cfg(not(all(feature = "ghostty-vt", unix)))]
        let _ = signals;
        match self {
            Self::Alacritty(engine) => engine.feed(bytes),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.feed(bytes, signals),
        }
        Ok(())
    }
    pub(super) fn key(&mut self, key: Key<'_>) -> Result<Vec<u8>> {
        match self {
            Self::Alacritty(engine) => {
                Ok(crate::encode(key, engine.app_cursor()).unwrap_or_default())
            }
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.key(key),
        }
    }
    pub(super) fn bracketed_paste(&self) -> bool {
        match self {
            Self::Alacritty(engine) => engine.bracketed_paste(),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.bracketed_paste(),
        }
    }
    pub(super) fn resize(&mut self, grid: Grid) -> Result<()> {
        match self {
            Self::Alacritty(engine) => engine.resize(grid),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.resize(grid)?,
        }
        Ok(())
    }
    pub(super) fn set_base(&mut self, base: Base) -> Result<()> {
        #[cfg(not(all(feature = "ghostty-vt", unix)))]
        let _ = base;
        match self {
            Self::Alacritty(_) => {}
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.set_base(base)?,
        }
        Ok(())
    }
    pub(super) fn scroll(&mut self, lines: Option<i32>) {
        match self {
            Self::Alacritty(engine) => engine.scroll(lines),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.scroll(lines),
        }
    }
    pub(super) fn snapshot(&mut self, base: Base) -> Result<Screen> {
        match self {
            Self::Alacritty(engine) => Ok(engine.snapshot(base)),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.snapshot(base),
        }
    }
    pub(super) fn maintain(&mut self) -> Result<()> {
        match self {
            Self::Alacritty(_) => {}
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.maintain()?,
        }
        Ok(())
    }
    pub(super) fn maintenance_due(&self) -> Option<Instant> {
        match self {
            Self::Alacritty(_) => None,
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.maintenance_due(),
        }
    }
    pub(super) fn render_allowed(&self) -> bool {
        match self {
            Self::Alacritty(_) => true,
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(engine) => engine.render_allowed(),
        }
    }
}
