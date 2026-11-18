use std::ops::Range;

#[derive(Clone, Copy)]
enum CharacterKind {
    Word,
    Whitespace,
    Newline,
    Other,
}

// Word selection in native input/selectable text and inline Markdown uses the same policy.
// Script ranges follow the GPUI line wrapper; × and ÷ are symbols inside Latin-1.
pub(crate) fn is_word_char(c: char) -> bool {
    match c {
        '×' | '÷' => false,
        '_'
        | '\u{00C0}'..='\u{024F}'
        | '\u{0400}'..='\u{04FF}'
        | '\u{1E00}'..='\u{1EFF}'
        | '\u{0300}'..='\u{036F}' => true,
        _ => c.is_ascii_alphanumeric(),
    }
}

impl From<char> for CharacterKind {
    fn from(character: char) -> Self {
        if is_word_char(character) {
            Self::Word
        } else if matches!(character, '\n' | '\r') {
            Self::Newline
        } else if character.is_whitespace() {
            Self::Whitespace
        } else {
            Self::Other
        }
    }
}

pub(crate) fn word_range_from_chars(
    offset: usize,
    character: char,
    previous: impl Iterator<Item = char>,
    following: impl Iterator<Item = char>,
) -> Range<usize> {
    let kind = CharacterKind::from(character);
    let connects = |next| {
        matches!(
            (kind, CharacterKind::from(next)),
            (CharacterKind::Word, CharacterKind::Word)
                | (CharacterKind::Whitespace, CharacterKind::Whitespace)
        )
    };
    let start = previous
        .take(128)
        .take_while(|character| connects(*character))
        .fold(offset, |offset, character| offset - character.len_utf8());
    let end = following
        .take(128)
        .take_while(|character| connects(*character))
        .fold(offset + character.len_utf8(), |offset, character| {
            offset + character.len_utf8()
        });
    start..end
}

pub(crate) fn word_range_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let offset = clip_offset_left(text, offset);
    let character = text[offset..].chars().next()?;
    let end = offset + character.len_utf8();
    Some(word_range_from_chars(
        offset,
        character,
        text[..offset].chars().rev(),
        text[end..].chars(),
    ))
}

pub(crate) fn line_range_at(text: &str, offset: usize) -> Range<usize> {
    let offset = clip_offset_left(text, offset);
    let start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |newline| offset + newline);
    start..end
}

fn clip_offset_left(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
