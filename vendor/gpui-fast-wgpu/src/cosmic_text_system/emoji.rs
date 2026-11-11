//! Choose a supported color face by Unicode presentation, rather than letting an
//! unrelated UI font's monochrome coverage override emoji presentation.
use super::*;
use unicode_properties::{EmojiStatus, UnicodeEmoji};

impl CosmicTextSystemState {
    pub(super) fn preferred_emoji_font(&mut self) -> Option<(FontId, SharedString)> {
        if let Some(cached) = &self.preferred_emoji_font {
            return cached.clone();
        }
        let db = self.font_system.db();
        let face = db
            .faces()
            .filter_map(|face| {
                if !check_is_known_emoji_font(&face.post_script_name)
                    || db.with_face_data(face.id, has_supported_color_bitmap) != Some(true)
                {
                    return None;
                }
                Some((
                    face.id,
                    SharedString::from(face.families.first()?.0.clone()),
                ))
            })
            .last();
        let resolved = face
            .and_then(|(id, family)| self.font_id_for_cosmic_id(id).ok().map(|id| (id, family)));
        self.preferred_emoji_font = Some(resolved.clone());
        resolved
    }
}

fn wants_emoji(grapheme: &str) -> bool {
    let Some(base) = grapheme.chars().next() else {
        return false;
    };
    if grapheme.contains('\u{fe0e}') {
        return false; // Explicit text presentation always wins.
    }
    if base.is_ascii_digit() || matches!(base, '#' | '*') {
        // Emoji=Yes includes ordinary ASCII digits. Only keycap sequences
        // request their color presentation, never bare digits or digits + VS16.
        return matches!(
            grapheme.get(base.len_utf8()..),
            Some("\u{20e3}" | "\u{fe0f}\u{20e3}")
        );
    }
    if grapheme.contains('\u{fe0f}') {
        return base.is_emoji_char();
    }
    let status = base.emoji_status();
    if matches!(status, EmojiStatus::EmojiModifierBase)
        && grapheme.chars().skip(1).any(|ch| {
            matches!(
                ch.emoji_status(),
                EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
            )
        })
    {
        // A valid modifier sequence (e.g. victory hand + skin tone) requests
        // emoji even when the unmodified base defaults to text presentation.
        return true;
    }
    matches!(
        status,
        EmojiStatus::EmojiPresentation
            | EmojiStatus::EmojiPresentationAndModifierBase
            | EmojiStatus::EmojiPresentationAndEmojiComponent
            | EmojiStatus::EmojiPresentationAndModifierAndEmojiComponent
    )
}

/// Emit coalesced, source-aligned spans without allocating a temporary list.
/// If the selected color font lacks a base, retain the normal fallback path.
pub(super) fn presentation_spans(
    text: &str,
    range: Range<usize>,
    covers: impl Fn(char) -> bool,
    mut emit: impl FnMut(Range<usize>, bool),
) {
    if text[range.clone()].is_ascii() {
        emit(range, false);
        return;
    }
    let mut start = range.start;
    let mut previous = None;
    for (offset, grapheme) in text[range.clone()].grapheme_indices(true) {
        let emoji = wants_emoji(grapheme) && grapheme.chars().next().is_some_and(&covers);
        if previous.is_some_and(|previous| previous != emoji) {
            emit(start..range.start + offset, previous.unwrap());
            start = range.start + offset;
        }
        previous = Some(emoji);
    }
    if let Some(emoji) = previous {
        emit(start..range.end, emoji);
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{IBM_PLEX, LILEX, NOTO_EMOJI};
    use super::*;

    /// Give the existing outline fixture emoji coverage. No installed fonts or
    /// generated binary assets are needed: only its cmap directory is replaced.
    fn outline_with_emoji_coverage() -> Vec<u8> {
        let font = swash::FontRef::from_index(IBM_PLEX, 0).unwrap();
        let charmap = font.charmap();
        let mut mappings: Vec<_> = (' '..='~')
            .map(|ch| (ch as u32, charmap.map(ch) as u32))
            .filter(|(_, glyph)| *glyph != 0)
            .collect();
        for ch in ['😄', '☺', '☀', '❤', '©', '👍', '👨', '🇷'] {
            mappings.push((ch as u32, charmap.map('A') as u32));
        }
        mappings.sort_unstable();
        let mut cmap = Vec::new();
        for value in [0u16, 1, 3, 10] {
            cmap.extend(value.to_be_bytes());
        }
        cmap.extend(12u32.to_be_bytes());
        for value in [12u16, 0] {
            cmap.extend(value.to_be_bytes());
        }
        for value in [16 + mappings.len() as u32 * 12, 0, mappings.len() as u32] {
            cmap.extend(value.to_be_bytes());
        }
        for (ch, glyph) in mappings {
            for value in [ch, ch, glyph] {
                cmap.extend(value.to_be_bytes());
            }
        }
        let mut bytes = IBM_PLEX.to_vec();
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        let offset = bytes.len() as u32;
        let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        for record in bytes[12..12 + count * 16].chunks_exact_mut(16) {
            if &record[..4] == b"cmap" {
                record[8..12].copy_from_slice(&offset.to_be_bytes());
                record[12..16].copy_from_slice(&(cmap.len() as u32).to_be_bytes());
            }
        }
        bytes.extend(cmap);
        bytes
    }

    #[test]
    fn emoji_presentation_overrides_primary_outline_coverage_and_rasterizes_color() -> Result<()> {
        let text_system = CosmicTextSystem::new_without_system_fonts("IBM Plex Sans");
        text_system.add_fonts(vec![Cow::Owned(outline_with_emoji_coverage())])?;
        let primary = text_system.font_id(&gpui::font("IBM Plex Sans"))?;
        assert!(charmap_covers(
            &text_system.0.read().loaded_fonts,
            primary,
            '😄'
        ));
        // Resolve once before registration, exercising invalidation of a cached miss.
        assert!(text_system.0.write().preferred_emoji_font().is_none());
        let without_color = text_system.layout_line(
            "😄",
            gpui::px(16.),
            &[FontRun {
                len: "😄".len(),
                font_id: primary,
            }],
        );
        assert_eq!(without_color.runs.len(), 1);
        assert_eq!(without_color.runs[0].font_id, primary);
        assert!(!without_color.runs[0].glyphs[0].is_emoji);
        text_system.add_fonts(vec![Cow::Borrowed(NOTO_EMOJI)])?;

        for sample in ["😄", "☀️", "©️", "👍🏽", "✌🏽", "👨‍👩‍👧‍👦", "🇷🇺", "1️⃣", "1\u{20e3}"]
        {
            let layout = text_system.layout_line(
                sample,
                gpui::px(16.),
                &[FontRun {
                    len: sample.len(),
                    font_id: primary,
                }],
            );
            let glyphs: Vec<_> = layout
                .runs
                .iter()
                .flat_map(|run| run.glyphs.iter().map(move |glyph| (run.font_id, glyph)))
                .collect();
            assert_eq!(glyphs.len(), 1, "whole emoji cluster: {sample}");
            let (font_id, glyph) = glyphs[0];
            assert_ne!(font_id, primary, "primary outline must not win: {sample}");
            assert!(glyph.is_emoji, "color classification: {sample}");
            let params = RenderGlyphParams {
                font_id,
                glyph_id: glyph.id,
                font_size: gpui::px(16.),
                subpixel_variant: point(0, 0),
                scale_factor: 1.,
                is_emoji: true,
                subpixel_rendering: false,
                dilation: 0,
            };
            let bounds = text_system.glyph_raster_bounds(&params)?;
            let (_, bitmap) = text_system.rasterize_glyph(&params, bounds)?;
            assert!(
                bitmap.chunks_exact(4).any(|pixel| pixel[3] != 0),
                "visible bitmap: {sample}"
            );
            // Some valid color-font glyphs (e.g. copyright) are intrinsically gray.
            if sample == "😄" {
                assert!(
                    bitmap.chunks_exact(4).any(
                        |pixel| pixel[3] != 0 && (pixel[0] != pixel[1] || pixel[1] != pixel[2])
                    ),
                    "visible RGB: {sample}"
                );
            }
        }
        for sample in ["A1#*Z", "😄\u{fe0e}", "☺", "☀", "❤", "©", "1\u{fe0f}"] {
            let layout = text_system.layout_line(
                sample,
                gpui::px(16.),
                &[FontRun {
                    len: sample.len(),
                    font_id: primary,
                }],
            );
            assert!(!layout.runs.is_empty());
            for run in layout.runs {
                assert_eq!(run.font_id, primary, "text presentation: {sample}");
                assert!(run.glyphs.iter().all(|glyph| !glyph.is_emoji));
            }
        }
        Ok(())
    }

    #[test]
    fn emoji_presentation_keeps_clusters_whole_across_named_fallback_spans() -> Result<()> {
        let text_system = CosmicTextSystem::new_without_system_fonts("IBM Plex Sans");
        text_system.add_fonts(vec![
            Cow::Owned(outline_with_emoji_coverage()),
            Cow::Borrowed(LILEX),
            Cow::Borrowed(NOTO_EMOJI),
        ])?;
        let mut font = gpui::font("IBM Plex Sans");
        font.fallbacks = Some(FontFallbacks::from_fonts(vec!["Lilex".into()]));
        let primary = text_system.font_id(&font)?;
        let fallback = {
            let state = text_system.0.read();
            let properties = state.font_match_properties(primary).unwrap();
            assert_eq!(properties.fallback_chain.len(), 1);
            // The primary covers cluster bases, but not their dependent parts.
            for ch in ['👍', '👨', '🇷', '1'] {
                assert!(charmap_covers(&state.loaded_fonts, primary, ch));
            }
            for ch in ['🏽', '\u{200d}', '🇺', '\u{20e3}'] {
                assert!(!charmap_covers(&state.loaded_fonts, primary, ch));
            }
            properties.fallback_chain[0].0
        };
        // Greek forces named fallback spans on either side of emoji clusters.
        let text = "Aα😄👍🏽👨‍👩‍👧‍👦🇷🇺1️⃣βZ";
        let layout = text_system.layout_line(
            text,
            gpui::px(16.),
            &[FontRun {
                len: text.len(),
                font_id: primary,
            }],
        );
        let mut glyph_count = 0;
        for run in layout.runs {
            for glyph in run.glyphs {
                glyph_count += 1;
                match text[glyph.index..].chars().next().unwrap() {
                    'A' | 'Z' => assert_eq!(run.font_id, primary),
                    'α' | 'β' => assert_eq!(run.font_id, fallback),
                    _ => assert!(glyph.is_emoji, "complete color cluster at {}", glyph.index),
                }
            }
        }
        assert_eq!(
            glyph_count, 9,
            "two Latin + two Greek + five emoji clusters"
        );
        Ok(())
    }

    #[test]
    fn emoji_presentation_preserves_unavailable_color_fallback_and_grapheme_ranges() {
        let text = "A😄☀️👍🏽👨‍👩‍👧‍👦🇷🇺1️⃣Z";
        let mut spans = Vec::new();
        presentation_spans(
            text,
            0..text.len(),
            |_| true,
            |range, emoji| {
                spans.push((&text[range], emoji));
            },
        );
        assert_eq!(spans, [("A", false), ("😄☀️👍🏽👨‍👩‍👧‍👦🇷🇺1️⃣", true), ("Z", false)]);
        spans.clear();
        presentation_spans(
            text,
            0..text.len(),
            |_| false,
            |range, emoji| {
                spans.push((&text[range], emoji));
            },
        );
        assert_eq!(spans, [(text, false)]);
    }
}
