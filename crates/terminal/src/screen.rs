//! Immutable visible-screen contract, independent of the VT engine.

/// The host theme's default colors; cells without an explicit color use them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Base {
    pub foreground: [u8; 3],
    pub background: [u8; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub foreground: [u8; 3],
    /// `None` keeps the view's own background.
    pub background: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
}

/// Consecutive cells of one row sharing a style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub column: usize,
    /// Number of grid columns the run covers (wide glyphs take two).
    pub width: usize,
    pub text: String,
    pub style: Style,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub columns: usize,
    pub rows: Vec<Vec<Run>>,
    /// Row and column of the cursor when it is visible on screen.
    pub cursor: Option<(usize, usize)>,
    /// Lines scrolled back from the live screen.
    pub scrolled: usize,
}

impl Screen {
    #[cfg(test)]
    pub fn text(&self) -> String {
        let lines: Vec<String> = self
            .rows
            .iter()
            .map(|row| row.iter().map(|run| run.text.as_str()).collect())
            .collect();
        lines.join("\n")
    }
}
