use chrono::{Datelike, Local, TimeZone, Timelike};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::conversations::ConversationSummary;

pub fn session_title(session: &ConversationSummary) -> String {
    session
        .preview
        .as_deref()
        .map(str::trim)
        .filter(|preview| !preview.is_empty())
        .map(|preview| clip(preview, 80))
        .unwrap_or_else(|| "Новая сессия".to_owned())
}

pub fn relative_time(updated_at_ms: Option<u64>) -> String {
    let Some(updated) = updated_at_ms else {
        return String::new();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(updated);
    let seconds = now.saturating_sub(updated) / 1000;
    match seconds {
        0..60 => "сейчас".to_owned(),
        60..3600 => format!("{} мин", seconds / 60),
        3600..86_400 => format!("{} ч", seconds / 3600),
        _ => format!("{} д", seconds / 86_400),
    }
}

/// First line of `text`, cut to `max_chars`.
pub fn clip(text: &str, max_chars: usize) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let line = line.trim();
    line.chars().take(max_chars).collect()
}

/// The first lines of `text`, at most `max_chars` of it, without a mark of
/// what is left out: the caller fades the end out instead.
pub fn peek_block(text: &str, max_lines: usize, max_chars: usize) -> String {
    let mut out = String::new();
    for line in text.lines().take(max_lines) {
        if out.len() + line.len() > max_chars {
            let room = max_chars.saturating_sub(out.len());
            out.push_str(&line[..line.floor_char_boundary(room)]);
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.truncate(out.trim_end().len());
    out
}

/// Leading part of a long tool result, kept short enough for the thread.
pub fn clip_block(text: &str, max_lines: usize, max_chars: usize) -> String {
    let mut out = String::new();
    let mut lines = 0;
    for line in text.lines() {
        if lines == max_lines || out.len() + line.len() > max_chars {
            // A single overlong first line is cut rather than hidden.
            if lines == 0 {
                let cut = line.floor_char_boundary(max_chars);
                out.push_str(&line[..cut]);
                out.push('\n');
                lines = 1;
            }
            let rest = text.lines().count() - lines;
            if rest == 0 {
                out.truncate(out.trim_end().len());
                return out;
            }
            out.push_str(&format!("Ещё строк: {rest}"));
            return out;
        }
        out.push_str(line);
        out.push('\n');
        lines += 1;
    }
    out.truncate(out.trim_end().len());
    out
}

const PREVIEW_LINES: usize = 8;
const PREVIEW_CHARS: usize = 2_000;

/// Shared settings and extension editor preview. None keeps short text editable;
/// long text copies only its visible prefix and never visits the hidden tail.
pub fn editor_preview(chars: impl Iterator<Item = char>) -> Option<String> {
    let mut preview = String::new();
    let mut lines = 1;
    for (count, ch) in chars.take(PREVIEW_CHARS + 1).enumerate() {
        if count == PREVIEW_CHARS || (ch == '\n' && lines == PREVIEW_LINES) {
            preview.truncate(preview.trim_end().len());
            return Some(preview);
        }
        if ch == '\n' {
            lines += 1;
        }
        preview.push(ch);
    }
    None
}

const MONTHS: [&str; 12] = [
    "янв.",
    "февр.",
    "мар.",
    "апр.",
    "мая",
    "июн.",
    "июл.",
    "авг.",
    "сент.",
    "окт.",
    "нояб.",
    "дек.",
];

/// Local date and time like "11 окт., 14:05".
pub fn local_time(seconds: u64) -> String {
    let Some(time) = i64::try_from(seconds)
        .ok()
        .and_then(|seconds| Local.timestamp_opt(seconds, 0).single())
    else {
        return seconds.to_string();
    };
    format!(
        "{} {}, {:02}:{:02}",
        time.day(),
        MONTHS[time.month0() as usize],
        time.hour(),
        time.minute()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_keeps_utf8_without_an_artificial_ellipsis() {
        assert_eq!(clip("hello", 10), "hello");
        assert_eq!(clip("héllo world", 5), "héllo");
        assert_eq!(clip("\nfirst\nsecond", 10), "first");
    }

    #[test]
    fn peek_block_keeps_the_first_lines() {
        assert_eq!(peek_block("a\nb\nc", 2, 100), "a\nb");
        assert_eq!(peek_block("abcdef\nz", 5, 3), "abc");
        assert_eq!(peek_block("ab\ncdef", 5, 4), "ab\nc");
    }

    #[test]
    fn clip_block_counts_hidden_lines() {
        assert_eq!(clip_block("a\nb\nc", 2, 100), "a\nb\nЕщё строк: 1");
        assert_eq!(clip_block("a\nb\n", 5, 100), "a\nb");
        assert_eq!(clip_block("abcdef", 5, 3), "abc");
        assert_eq!(clip_block("abcdef\nz", 5, 3), "abc\nЕщё строк: 1");
    }

    #[test]
    fn short_values_stay_editable_and_long_previews_are_utf8_bounded() {
        assert_eq!(editor_preview("one\ntwo".chars()), None);
        assert_eq!(
            editor_preview(["line"; PREVIEW_LINES].join("\n").chars()),
            None
        );
        assert_eq!(
            editor_preview(["line"; PREVIEW_LINES + 1].join("\n").chars()),
            Some(["line"; PREVIEW_LINES].join("\n"))
        );
        let text = "я".repeat(PREVIEW_CHARS + 100_000);
        let preview = editor_preview(text.chars()).unwrap();
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
    }

    #[test]
    fn render_preview_stops_before_reading_the_hidden_tail() {
        let mut read = 0;
        let chars = std::iter::from_fn(|| {
            read += 1;
            assert!(read <= PREVIEW_CHARS + 1, "read the hidden tail");
            Some('я')
        });
        assert_eq!(
            editor_preview(chars).unwrap().chars().count(),
            PREVIEW_CHARS
        );
        assert_eq!(read, PREVIEW_CHARS + 1);
        let mut read = 0;
        let chars = std::iter::from_fn(|| {
            read += 1;
            assert!(read <= PREVIEW_LINES * 2, "read beyond the line preview");
            Some(if read % 2 == 0 { '\n' } else { 'x' })
        });
        assert_eq!(editor_preview(chars), Some(["x"; PREVIEW_LINES].join("\n")));
        assert_eq!(read, PREVIEW_LINES * 2);
    }
}
