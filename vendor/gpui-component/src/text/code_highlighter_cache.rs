//! Shared thread-local code-block admission: bounded text and grammar states.
use crate::highlighter::SyntaxHighlighter;
use gpui::SharedString;
use std::collections::VecDeque;

const MAX_ENTRIES: usize = 8;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_BLOCK_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub(super) struct CodeHighlighterCache {
    generation: u64,
    entries: VecDeque<(SharedString, usize, SyntaxHighlighter)>,
}

impl CodeHighlighterCache {
    pub(super) fn prepare(
        &mut self,
        name: SharedString,
        generation: u64,
        bytes: usize,
    ) -> Option<&mut SyntaxHighlighter> {
        if generation != self.generation {
            self.entries.clear();
            self.generation = generation;
        }
        if bytes > MAX_BLOCK_BYTES {
            return None;
        }
        let retained = self
            .entries
            .iter()
            .position(|(key, _, _)| key == &name)
            .and_then(|index| self.entries.remove(index));
        while self.entries.len() >= MAX_ENTRIES
            || self.entries.iter().map(|(_, size, _)| size).sum::<usize>() + bytes > MAX_BYTES
        {
            self.entries.pop_front();
        }
        let highlighter = retained
            .map(|(_, _, highlighter)| highlighter)
            .unwrap_or_else(|| SyntaxHighlighter::new(name.as_ref()));
        self.entries.push_back((name, bytes, highlighter));
        self.entries
            .back_mut()
            .map(|(_, _, highlighter)| highlighter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_text_and_entry_admission_remain_bounded_and_grammar_generation_invalidates() {
        let mut cache = CodeHighlighterCache::default();
        for index in 0..100 {
            cache
                .prepare(format!("test-{index}").into(), 1, MAX_BLOCK_BYTES)
                .unwrap();
            assert!(cache.entries.len() <= MAX_ENTRIES);
            assert!(cache.entries.iter().map(|(_, size, _)| size).sum::<usize>() <= MAX_BYTES);
        }
        assert!(
            cache
                .prepare("too-large".into(), 1, MAX_BLOCK_BYTES + 1)
                .is_none()
        );
        cache.prepare("new-generation".into(), 2, 1).unwrap();
        assert_eq!(cache.entries.len(), 1);
    }
    #[cfg(feature = "tree-sitter")]
    #[test]
    fn audit_same_name_grammar_replacement_recreates_the_cached_parser() {
        use crate::highlighter::{GrammarConfig, LanguageRegistry};
        use gpui_base::input::{InputEdit, Point};
        use ropey::Rope;
        let registry = LanguageRegistry::singleton();
        let name = "audit-grammar-replacement";
        registry.register(name, &GrammarConfig::plain(name));
        let mut cache = CodeHighlighterCache::default();
        let text = Rope::from_str("123");
        let parse = |highlighter: &mut SyntaxHighlighter| {
            highlighter.update_input(
                Some(InputEdit {
                    start_byte: 0,
                    old_end_byte: 0,
                    new_end_byte: 3,
                    start_position: Point::new(0, 0),
                    old_end_position: Point::new(0, 0),
                    new_end_position: Point::new(0, 3),
                }),
                &text,
                None,
            );
        };
        let highlighter = cache
            .prepare(name.into(), registry.generation(), 3)
            .unwrap();
        parse(highlighter);
        assert!(highlighter.tree().is_none());
        let config = GrammarConfig::new(
            name,
            tree_sitter_json::LANGUAGE.into(),
            vec![],
            "(number) @number",
            "",
            "",
        );
        registry.register(name, &config);
        let highlighter = cache
            .prepare(name.into(), registry.generation(), 3)
            .unwrap();
        parse(highlighter);
        assert!(highlighter.tree().is_some());
    }
}
