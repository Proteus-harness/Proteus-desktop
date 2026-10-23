//! Syntax is parsed once for each source side, not for every visible row.
//! Colors use the same tree-sitter registry and theme as native code editors.
use crate::diff::{File, Kind};
use gpui_kit::base::input::Rope;
use gpui_kit::component::highlighter::{HighlightTheme, SyntaxHighlighter};
use gpui_kit::{HighlightStyle, SharedString};
use std::ops::Range;

pub(super) type Highlights = Vec<(Range<usize>, HighlightStyle)>;

pub(super) struct Syntax {
    before: Vec<Highlights>,
    after: Vec<Highlights>,
}

impl Syntax {
    pub fn new(file: &File, theme: &HighlightTheme) -> Self {
        Self {
            before: side(file, true, theme),
            after: side(file, false, theme),
        }
    }
    pub fn line(&self, index: usize, old: bool) -> &Highlights {
        if old {
            &self.before[index]
        } else {
            &self.after[index]
        }
    }
}

fn language(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name {
        "Cargo.lock" | "Pipfile" => "toml".into(),
        "Makefile" | "Dockerfile" | ".bashrc" | ".zshrc" | ".profile" => "bash".into(),
        _ => match name
            .rsplit_once('.')
            .map(|(_, s)| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("js" | "jsx" | "mjs" | "cjs") => "javascript".into(),
            Some("rs") => "rust".into(),
            Some("py" | "pyi") => "python".into(),
            Some("sh" | "zsh") => "bash".into(),
            Some("md" | "mdx") => "markdown".into(),
            Some("yml") => "yaml".into(),
            Some("jsonc") => "json".into(),
            Some("ts" | "mts" | "cts") => "typescript".into(),
            Some("h" | "hpp" | "cc" | "cxx" | "hh") => "cpp".into(),
            Some("htm") => "html".into(),
            Some(suffix) => suffix.to_owned(),
            None => "text".into(),
        },
    }
}

fn side(file: &File, old: bool, theme: &HighlightTheme) -> Vec<Highlights> {
    let mut source = String::new();
    let mut ranges = vec![None; file.lines.len()];
    for (index, line) in file.lines.iter().enumerate() {
        if line.kind == Kind::Hunk {
            source.push('\n');
            continue;
        }
        if line.kind == Kind::Note
            || (old && line.kind == Kind::Added)
            || (!old && line.kind == Kind::Removed)
        {
            continue;
        }
        let start = source.len();
        source.push_str(&line.text.replace('\t', "    "));
        ranges[index] = Some(start..source.len());
        source.push('\n');
    }
    let path = if old {
        file.old_path.as_deref().unwrap_or(&file.path)
    } else {
        &file.path
    };
    let mut highlighter = SyntaxHighlighter::new(&language(path));
    highlighter.update(None, &Rope::from_str(&source), None);
    let styles = highlighter.styles(&(0..source.len()), theme);
    let mut result = vec![Vec::new(); file.lines.len()];
    // Both sequences are ordered; do not rescan every token for every line.
    let mut at = 0;
    for (index, range) in ranges.into_iter().enumerate() {
        let Some(range) = range else { continue };
        while at < styles.len() && styles[at].0.end <= range.start {
            at += 1;
        }
        for (span, style) in &styles[at..] {
            if span.start >= range.end {
                break;
            }
            let start = span.start.max(range.start) - range.start;
            let end = span.end.min(range.end) - range.start;
            if start < end {
                result[index].push((start..end, *style));
            }
        }
    }
    result
}

pub(super) fn code(
    id: gpui_kit::ElementId,
    text: SharedString,
    highlights: &Highlights,
) -> gpui_kit::Div {
    use gpui_kit::base::SelectableText;
    use gpui_kit::*;
    // The native selection run owns hit testing and copy. The transparent
    // foreground allows the styled text above it to retain syntax colors.
    div()
        .relative()
        .child(
            SelectableText::new(id, text.clone()).text_style(TextStyleRefinement {
                color: Some(transparent_black()),
                ..Default::default()
            }),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .child(StyledText::new(text).with_highlights(highlights.clone())),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_highlights_keep_each_source_side_and_utf8_byte_ranges() {
        let files = crate::diff::parse(
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-let old = \"раньше\";\n+let new = \"теперь\";\n",
            &[],
        );
        let syntax = Syntax::new(&files[0], &HighlightTheme::default_dark());
        for (index, old) in [(1, true), (2, false)] {
            let line = &files[0].lines[index].text;
            let highlights = syntax.line(index, old);
            assert!(highlights.iter().any(|(_, s)| s.color.is_some()));
            assert!(
                highlights
                    .iter()
                    .all(|(r, _)| line.is_char_boundary(r.start)
                        && line.is_char_boundary(r.end)
                        && r.end <= line.len())
            );
            assert!(syntax.line(index, !old).is_empty());
        }
    }
}
