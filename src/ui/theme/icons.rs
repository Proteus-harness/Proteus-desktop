use gpui_kit::component::{IconTone, ThemeMode};
use gpui_kit::{Hsla, rgb};

pub(in crate::ui) fn color(tone: IconTone, mode: ThemeMode) -> Hsla {
    let (dark, light) = match tone {
        IconTone::Normal => (0xe4e4e7, 0x343438),
        IconTone::Muted => (0xa1a1aa, 0x606068),
        IconTone::Selected => (0xffffff, 0x18181b),
        IconTone::Accent => (0x93c5fd, 0x1d4ed8),
        IconTone::Success => (0x74d99f, 0x197346),
        IconTone::Warning => (0xf2c76e, 0x895a08),
        IconTone::Danger => (0xfb9292, 0xb52f38),
        IconTone::Custom(color) => return color,
    };
    rgb(if mode.is_dark() { dark } else { light }).into()
}
