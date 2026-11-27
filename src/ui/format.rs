use chrono::Utc;

use crate::conversations::ConversationSummary;
use crate::settings::catalog::plural;

pub fn session_title(session: &ConversationSummary) -> String {
    clip(crate::sessions::preview_title(session), 80)
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
    let inspect = max_chars.saturating_add(max_lines).saturating_add(4);
    let text = &text[..text.floor_char_boundary(inspect)];
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
    let mut input = text.lines();
    while let Some(line) = input.next() {
        if lines == max_lines || out.len() + line.len() > max_chars {
            // A single overlong first line is cut rather than hidden.
            if lines == 0 {
                let cut = line.floor_char_boundary(max_chars);
                out.push_str(&line[..cut]);
                out.push('\n');
            }
            let rest = input.count() + usize::from(lines != 0);
            if rest == 0 {
                out.truncate(out.trim_end().len());
                out.push_str("\n… Текст обрезан");
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
    for (count, ch) in chars.enumerate() {
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

/// Time elapsed since the last change, displayed when the hint opens.
pub fn relative_age(updated_at_ms: u64) -> String {
    let now_ms = u64::try_from(Utc::now().timestamp_millis()).unwrap_or_default();
    format_age(now_ms.saturating_sub(updated_at_ms) / 1_000)
}

fn format_age(seconds: u64) -> String {
    let (count, one, few, many) = match seconds {
        0..=59 => return "только что".to_owned(),
        60..=3_599 => (seconds / 60, "минуту", "минуты", "минут"),
        3_600..=86_399 => (seconds / 3_600, "час", "часа", "часов"),
        _ => (seconds / 86_400, "день", "дня", "дней"),
    };
    format!("{} назад", plural(count as usize, one, few, many))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_age_uses_completed_units_and_russian_forms() {
        assert_eq!(format_age(59), "только что");
        assert_eq!(format_age(60), "1 минуту назад");
        assert_eq!(format_age(3_599), "59 минут назад");
        assert_eq!(format_age(3_600), "1 час назад");
        assert_eq!(format_age(7_200), "2 часа назад");
        assert_eq!(format_age(86_399), "23 часа назад");
        assert_eq!(format_age(86_400), "1 день назад");
        assert_eq!(format_age(3 * 86_400), "3 дня назад");
        assert_eq!(format_age(11 * 86_400), "11 дней назад");
    }

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
        assert_eq!(clip_block("abcdef", 5, 3), "abc\n… Текст обрезан");
        assert_eq!(clip_block("abcdef\nz", 5, 3), "abc\nЕщё строк: 1");
    }

    #[test]
    fn values_at_the_preview_limits_stay_editable() {
        assert_eq!(editor_preview("one\ntwo".chars()), None);
        assert_eq!(
            editor_preview(["line"; PREVIEW_LINES].join("\n").chars()),
            None
        );
        assert_eq!(
            editor_preview(["line"; PREVIEW_LINES + 1].join("\n").chars()),
            Some(["line"; PREVIEW_LINES].join("\n"))
        );
        assert_eq!(
            editor_preview(std::iter::repeat_n('я', PREVIEW_CHARS)),
            None
        );
    }

    #[test]
    fn render_preview_stops_before_reading_the_hidden_tail() {
        let mut read = 0;
        let chars = std::iter::from_fn(|| {
            read += 1;
            assert!(read <= PREVIEW_CHARS + 1, "read the hidden tail");
            Some('я')
        });
        assert!(
            editor_preview(chars)
                .unwrap()
                .chars()
                .eq(std::iter::repeat_n('я', PREVIEW_CHARS))
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
