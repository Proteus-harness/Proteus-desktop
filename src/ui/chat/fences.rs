// Адаптировано из Zed: crates/agent_ui/src/conversation_view/thread_view.rs
// (recognize a complete fence before replacing its ordinary markdown view).
//! Offset-preserving, top-level fenced content. CommonMark recognizes an
//! unfinished fence as code too, so its closing line is checked explicitly.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Fence {
    pub ordinal: usize,
    pub range: Range<usize>,
    pub source: Range<usize>,
    pub language: String,
    pub extension: &'static str,
}

pub(super) fn registered(text: &str, languages: &[(String, &'static str)]) -> Vec<Fence> {
    if languages.is_empty() || !has_closing_line(text) {
        return Vec::new();
    }
    let Ok(markdown::mdast::Node::Root(root)) =
        markdown::to_mdast(text, &markdown::ParseOptions::default())
    else {
        return Vec::new();
    };
    root.children
        .into_iter()
        .filter_map(|node| match node {
            markdown::mdast::Node::Code(code) => Some(code),
            _ => None,
        })
        .enumerate()
        .filter_map(|(ordinal, code)| {
            let language = code.lang?.to_ascii_lowercase();
            let extension = languages.iter().find(|(name, _)| *name == language)?.1;
            let position = code.position?;
            let range = position.start.offset..position.end.offset;
            let raw = text.get(range.clone())?;
            let source = closed_source(raw)?;
            Some(Fence {
                ordinal,
                source: range.start + source.start..range.start + source.end,
                range,
                language,
                extension,
            })
        })
        .collect()
}

/// A streamed append cannot change a fence whose closing line already
/// ended. Reparse only the following markdown, never the large rich source
/// already handed to a retained view. Edits/provider changes use a full parse.
pub(super) fn appended(
    text: &str,
    previous: &str,
    found: &[Fence],
    languages: &[(String, &'static str)],
) -> Vec<Fence> {
    if text.starts_with(previous)
        && let Some(last) = found.last()
        && matches!(text.as_bytes().get(last.range.end), Some(b'\r' | b'\n'))
    {
        let boundary = last.range.end;
        let mut result = found.to_vec();
        result.extend(
            registered(&text[boundary..], languages)
                .into_iter()
                .map(|mut fence| {
                    fence.ordinal += last.ordinal + 1;
                    fence.range = fence.range.start + boundary..fence.range.end + boundary;
                    fence.source = fence.source.start + boundary..fence.source.end + boundary;
                    fence
                }),
        );
        return result;
    }
    registered(text, languages)
}

// Before CommonMark sees a streaming code block, require a possible closing
// line. Scanning bytes is much cheaper than constructing an AST per delta.
fn has_closing_line(text: &str) -> bool {
    text.lines().skip(1).any(|line| {
        let trimmed = line.trim_start_matches(' ');
        if line.len() - trimmed.len() > 3 {
            return false;
        }
        let Some(marker @ (b'`' | b'~')) = trimmed.as_bytes().first().copied() else {
            return false;
        };
        let count = trimmed.bytes().take_while(|byte| *byte == marker).count();
        count >= 3 && trimmed[count..].trim().is_empty()
    })
}

fn closed_source(raw: &str) -> Option<Range<usize>> {
    let first_end = raw.find('\n')? + 1;
    let opening = raw[..first_end].trim_start_matches(' ');
    let marker = opening.as_bytes().first().copied()?;
    if marker != b'`' && marker != b'~' {
        return None;
    }
    let length = opening.bytes().take_while(|byte| *byte == marker).count();
    if length < 3 {
        return None;
    }
    let end = raw.trim_end_matches(['\r', '\n']);
    let closing_start = end.rfind('\n')? + 1;
    if closing_start < first_end {
        return None;
    }
    let closing = end[closing_start..].trim_start_matches(' ');
    let count = closing.bytes().take_while(|byte| *byte == marker).count();
    if count < length || !closing[count..].trim().is_empty() {
        return None;
    }
    Some(first_end..closing_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(text: &str) -> Vec<Fence> {
        registered(text, &[("figure".into(), "fixture")])
    }

    #[test]
    fn only_closed_registered_top_level_fences_are_replaced() {
        let text = "Сначала текст\n\n```figure\n{\"value\":1}\n```\n\n```rust\nx\n```\n\n```figure\nunfinished";
        let found = blocks(text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].source.clone()], "{\"value\":1}\n");
        assert_eq!(
            &text[found[0].range.clone()],
            "```figure\n{\"value\":1}\n```"
        );
        for nested in [
            "> ```figure\n> x\n> ```",
            "- item\n\n  ```figure\n  x\n  ```",
            "    ```figure\n    x\n    ```",
            "```figure\nx",
            "```figure\nx\n``",
        ] {
            assert!(blocks(nested).is_empty(), "{nested}");
        }
    }

    #[test]
    fn longer_and_tilde_fences_preserve_source_and_stable_offsets() {
        let text = "````figure\n```inner\nunchanged\n```\n````\n";
        let found = blocks(text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].source.clone()], "```inner\nunchanged\n```\n");
        let first = blocks("~~~figure\r\nx\r\n~~~~");
        assert_eq!(first.len(), 1);
        let appended = blocks("~~~figure\r\nx\r\n~~~~\n\nNew streamed text");
        assert_eq!(first, appended);
        assert!(blocks("```figure\nx\n``` is not a closing line").is_empty());
    }

    #[test]
    fn unicode_and_source_edits_keep_other_block_identity() {
        let before = "Привет 🌍\n\n```figure\nтекст 🦀\n```\n\n```figure\nвторой\n```";
        let after = before.replace("текст 🦀", "более длинный текст 🦀");
        let (old, new) = (blocks(before), blocks(&after));
        assert_eq!((old.len(), new.len()), (2, 2));
        assert_eq!(&before[old[0].source.clone()], "текст 🦀\n");
        assert_ne!(old[1].range.start, new[1].range.start);
        assert_eq!(old[1].ordinal, new[1].ordinal);
        assert_eq!(
            &before[old[1].source.clone()],
            &after[new[1].source.clone()]
        );
    }

    #[test]
    fn provider_changes_reconcile_an_already_present_fence() {
        let text = "```new-language\nunchanged source\n```";
        assert!(registered(text, &[]).is_empty());
        let one = vec![("new-language".into(), "one")];
        assert_eq!(registered(text, &one)[0].extension, "one");
        let both = vec![
            ("new-language".into(), "two"),
            ("new-language".into(), "one"),
        ];
        let rebound = registered(text, &both);
        assert_eq!(rebound[0].extension, "two");
        assert_eq!(rebound[0].ordinal, registered(text, &one)[0].ordinal);
        assert!(registered(text, &[]).is_empty());
    }

    #[test]
    fn streamed_appends_parse_only_tail_after_a_finished_closing_line() {
        let languages = vec![("figure".into(), "fixture")];
        let before = "```figure\nfirst\n```\n\nText";
        let after =
            format!("{before}\n\n```other\nnot registered\n```\n\n```figure\nsecond\n```\n");
        let retained = registered(before, &languages);
        assert_eq!(
            appended(&after, before, &retained, &languages),
            registered(&after, &languages)
        );
        // A closing marker at EOF can still become ordinary source when
        // the next delta adds text to that same line.
        let before = "```figure\nfirst\n```";
        let after = format!("{before}no longer a closing fence");
        assert!(appended(&after, before, &registered(before, &languages), &languages).is_empty());
        assert!(!has_closing_line(
            "```figure\nstreamed source without closing marker"
        ));
    }
}
