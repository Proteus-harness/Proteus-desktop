//! Convert the host key into Ghostty's maintained legacy/Kitty encoder.
use crate::Key;
use anyhow::Result;
use ghostty_vt::Terminal;
use ghostty_vt::key::{Action, Encoder, Event, Key as Code, Mods};

pub(super) struct KeyInput {
    code: Code,
    mods: Mods,
    text: Option<String>,
    unshifted: char,
}

impl KeyInput {
    pub(super) fn new(key: Key<'_>) -> Option<Self> {
        if key.key.len() > 64 || key.text.is_some_and(|text| text.len() > 4096) {
            return None;
        }
        // Use the established host key admission rules, including modifier-only keys.
        crate::encode(key, false)?;
        let mut mods = Mods::empty();
        mods.set(Mods::CTRL, key.ctrl);
        mods.set(Mods::ALT, key.alt);
        mods.set(Mods::SHIFT, key.shift);
        let unshifted = key
            .key
            .chars()
            .next()
            .filter(|_| key.key.chars().count() == 1)
            .unwrap_or('\0')
            .to_ascii_lowercase();
        Some(Self {
            code: code(key.key),
            mods,
            text: key.text.map(str::to_owned),
            unshifted,
        })
    }
    pub(super) fn reserved(&self) -> usize {
        self.text.as_ref().map_or(0, String::len) + 128
    }
    pub(super) fn encode(
        self,
        terminal: &Terminal,
        encoder: &mut Encoder,
        event: &mut Event,
    ) -> Result<Vec<u8>> {
        event
            .set_action(Action::Press)
            .set_key(self.code)
            .set_mods(self.mods)
            .set_consumed_mods(Mods::empty())
            .set_utf8(self.text)
            .set_unshifted_codepoint(self.unshifted);
        let mut bytes = Vec::new();
        encoder
            .set_options_from_terminal(terminal)
            .encode_to_vec(event, &mut bytes)?;
        Ok(bytes)
    }
}

fn code(name: &str) -> Code {
    match name {
        "up" => Code::ArrowUp,
        "down" => Code::ArrowDown,
        "left" => Code::ArrowLeft,
        "right" => Code::ArrowRight,
        "home" => Code::Home,
        "end" => Code::End,
        "insert" => Code::Insert,
        "delete" => Code::Delete,
        "pageup" => Code::PageUp,
        "pagedown" => Code::PageDown,
        "enter" => Code::Enter,
        "escape" => Code::Escape,
        "tab" => Code::Tab,
        "backspace" => Code::Backspace,
        "space" | " " => Code::Space,
        "`" => Code::Backquote,
        "\\" => Code::Backslash,
        "[" => Code::BracketLeft,
        "]" => Code::BracketRight,
        "," => Code::Comma,
        "." => Code::Period,
        "=" => Code::Equal,
        "-" => Code::Minus,
        "'" => Code::Quote,
        ";" => Code::Semicolon,
        "/" => Code::Slash,
        "f1" => Code::F1,
        "f2" => Code::F2,
        "f3" => Code::F3,
        "f4" => Code::F4,
        "f5" => Code::F5,
        "f6" => Code::F6,
        "f7" => Code::F7,
        "f8" => Code::F8,
        "f9" => Code::F9,
        "f10" => Code::F10,
        "f11" => Code::F11,
        "f12" => Code::F12,
        _ => {
            const LETTERS: [Code; 26] = [
                Code::A,
                Code::B,
                Code::C,
                Code::D,
                Code::E,
                Code::F,
                Code::G,
                Code::H,
                Code::I,
                Code::J,
                Code::K,
                Code::L,
                Code::M,
                Code::N,
                Code::O,
                Code::P,
                Code::Q,
                Code::R,
                Code::S,
                Code::T,
                Code::U,
                Code::V,
                Code::W,
                Code::X,
                Code::Y,
                Code::Z,
            ];
            const DIGITS: [Code; 10] = [
                Code::Digit0,
                Code::Digit1,
                Code::Digit2,
                Code::Digit3,
                Code::Digit4,
                Code::Digit5,
                Code::Digit6,
                Code::Digit7,
                Code::Digit8,
                Code::Digit9,
            ];
            let bytes = name.as_bytes();
            if bytes.len() != 1 {
                return Code::Unidentified;
            }
            match bytes[0].to_ascii_lowercase() {
                b'a'..=b'z' => LETTERS[(bytes[0].to_ascii_lowercase() - b'a') as usize],
                b'0'..=b'9' => DIGITS[(bytes[0] - b'0') as usize],
                _ => Code::Unidentified,
            }
        }
    }
}
