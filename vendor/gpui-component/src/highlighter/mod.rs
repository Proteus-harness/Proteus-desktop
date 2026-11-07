mod language_name;
mod status_colors;
pub use gpui_base::input::{
    Diagnostic, DiagnosticEntry, DiagnosticRelatedInformation, DiagnosticSet, DiagnosticSeverity,
    DiagnosticSummary, DiagnosticTag, RelatedInformation,
};
pub(crate) use language_name::language_name;

mod diagnostic_styles;
pub(crate) use diagnostic_styles::*;

#[cfg(feature = "tree-sitter")]
mod input_adapter;
#[cfg(feature = "tree-sitter")]
pub(crate) use input_adapter::input_highlighter_factory;

// The lighter highlighter, used when tree-sitter is not enabled.
#[cfg(all(feature = "syntect", not(feature = "tree-sitter")))]
mod syntect_backend;
#[cfg(all(feature = "syntect", not(feature = "tree-sitter")))]
pub(crate) use syntect_backend::{highlight_code, input_highlighter_factory};

#[cfg(not(any(feature = "tree-sitter", feature = "syntect")))]
pub(crate) fn input_highlighter_factory() -> gpui_base::input::InputHighlighterFactory {
    std::rc::Rc::new(|_| None)
}

// Native implementation with full tree-sitter support
#[cfg(feature = "tree-sitter")]
mod highlighter;
#[cfg(feature = "tree-sitter")]
mod languages;
#[cfg(feature = "tree-sitter")]
mod registry;

#[cfg(feature = "tree-sitter")]
pub(crate) use highlighter::parse_input_bytes;
#[cfg(feature = "tree-sitter")]
pub use highlighter::*;
#[cfg(feature = "tree-sitter")]
pub use languages::*;
#[cfg(feature = "tree-sitter")]
pub use registry::*;

// WASM stub implementation (no tree-sitter support or disabled)
#[cfg(not(feature = "tree-sitter"))]
mod wasm_stub;
#[cfg(not(feature = "tree-sitter"))]
pub use wasm_stub::*;

#[cfg(test)]
mod audit_regressions {
    use super::*;
    #[test]
    fn audit_theme_underline_and_dotted_editor_keys_survive_both_backends() {
        let style: ThemeStyle =
            serde_json::from_str(r#"{"color":null,"font_style":"underline","font_weight":null}"#)
                .unwrap();
        let highlight: gpui::HighlightStyle = style.into();
        assert!(highlight.underline.is_some());
        let mut value = serde_json::to_value(HighlightThemeStyle::default()).unwrap();
        value["editor.background"] = serde_json::to_value(gpui::hsla(0.25, 0.3, 0.4, 1.)).unwrap();
        let parsed: HighlightThemeStyle = serde_json::from_value(value).unwrap();
        let expected: gpui::Hsla =
            serde_json::from_value(serde_json::to_value(gpui::hsla(0.25, 0.3, 0.4, 1.)).unwrap())
                .unwrap();
        assert_eq!(parsed.editor_background, Some(expected));
    }
}
