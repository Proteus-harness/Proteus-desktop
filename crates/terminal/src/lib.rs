//! Headless PTY terminal with one selected VT engine and immutable visible snapshots.
mod alacritty;
mod alacritty_screen;
#[cfg(all(feature = "ghostty-vt", unix))]
mod ghostty;
mod keys;
mod screen;

use anyhow::Result;
pub use keys::{Key, encode};
pub use screen::{Base, Run, Screen};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    Title(Option<String>),
    Bell,
    Exited,
    Failed(String),
    InputRejected(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    pub columns: usize,
    pub rows: usize,
}
impl Grid {
    pub fn new(columns: usize, rows: usize) -> Self {
        // Bound the visible allocation as well as the PTY size.
        Self {
            columns: columns.clamp(2, 1000),
            rows: rows.clamp(1, 500),
        }
    }
}

pub struct Spawned {
    pub terminal: Terminal,
    pub signals: async_channel::Receiver<Signal>,
    pub redraw: async_channel::Receiver<()>,
}

pub enum Terminal {
    #[doc(hidden)]
    Alacritty(alacritty::Terminal),
    #[cfg(all(feature = "ghostty-vt", unix))]
    #[doc(hidden)]
    Ghostty(ghostty::Terminal),
}

impl Terminal {
    pub fn spawn(cwd: &Path, grid: Grid) -> Result<Spawned> {
        let grid = Grid::new(grid.columns, grid.rows);
        let engine = std::env::var("PROTEUS_TERMINAL_ENGINE").unwrap_or_default();
        let engine = if engine.is_empty() {
            if cfg!(all(feature = "ghostty-vt", unix)) {
                "ghostty"
            } else {
                "alacritty"
            }
        } else {
            engine.as_str()
        };
        match engine {
            "ghostty" => {
                #[cfg(all(feature = "ghostty-vt", unix))]
                {
                    return ghostty::Terminal::spawn(cwd, grid);
                }
                #[cfg(not(all(feature = "ghostty-vt", unix)))]
                anyhow::bail!("Эта сборка не содержит движок Ghostty");
            }
            "alacritty" => {}
            other => anyhow::bail!("Неизвестный движок терминала: {other}"),
        }
        alacritty::Terminal::spawn(cwd, grid)
    }
    /// Encode a host key using the selected engine's current terminal modes.
    pub fn key(&self, key: Key<'_>) -> bool {
        match self {
            Self::Alacritty(term) => match encode(key, term.app_cursor()) {
                Some(bytes) => {
                    term.write(bytes);
                    true
                }
                None => false,
            },
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(term) => term.key(key),
        }
    }
    pub fn write(&self, bytes: Vec<u8>) {
        match self {
            Self::Alacritty(term) => term.write(bytes),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(term) => term.write(bytes),
        }
    }
    pub fn paste(&self, text: &str) {
        match self {
            Self::Alacritty(term) => term.paste(text),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(term) => term.paste(text),
        }
    }
    pub fn app_cursor(&self) -> bool {
        match self {
            Self::Alacritty(term) => term.app_cursor(),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(term) => term.app_cursor(),
        }
    }
    pub fn resize(&mut self, grid: Grid) {
        let grid = Grid::new(grid.columns, grid.rows);
        match self {
            Self::Alacritty(term) => term.resize(grid),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(term) => term.resize(grid),
        }
    }
    pub fn scroll(&self, lines: i32) {
        match self {
            Self::Alacritty(term) => term.scroll(lines),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(term) => term.scroll(lines),
        }
    }
    pub fn snapshot(&self, base: Base) -> Screen {
        match self {
            Self::Alacritty(term) => term.snapshot(base),
            #[cfg(all(feature = "ghostty-vt", unix))]
            Self::Ghostty(term) => term.snapshot(base),
        }
    }
}

/// Finish owned Ghostty PTY cleanup after the UI event loop has returned.
/// The total wait is bounded to two seconds across all terminals.
/// Normal tab destruction remains nonblocking.
pub fn shutdown_all() {
    #[cfg(all(feature = "ghostty-vt", unix))]
    ghostty::shutdown_all();
}

fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        // A pasted end marker would let the text escape bracketed mode.
        let text = text.replace("\x1b[201~", "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn paste_uses_carriage_returns_and_brackets() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(
            paste_bytes("x\x1b[201~y", true),
            b"\x1b[200~xy\x1b[201~".to_vec()
        );
    }

    #[test]
    fn shell_output_reaches_the_screen() {
        let dir = std::env::temp_dir();
        let spawned = Terminal::spawn(&dir, Grid::new(40, 6)).expect("spawn");
        spawned
            .terminal
            .write(b"printf 'proteus-%s\\n' terminal\r".to_vec());
        let base = Base {
            foreground: [200, 200, 200],
            background: [0, 0, 0],
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let screen = spawned.terminal.snapshot(base);
            if screen.text().contains("proteus-terminal") {
                break;
            }
            assert!(Instant::now() < deadline, "no output:\n{}", screen.text());
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
