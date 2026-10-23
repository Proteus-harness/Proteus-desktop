//! The time of a message beside its row actions: short in the row, the full
//! date and time in the tooltip.

use chrono::{DateTime, Datelike as _, Local, TimeZone as _};
use gpui_kit::component::*;
use gpui_kit::*;

use crate::ui::tip::Tip as _;

/// The time of row `key` at `time` (Unix milliseconds), muted and small.
pub(super) fn message_time(key: u64, time: Option<i64>, cx: &App) -> Option<AnyElement> {
    let time = Local.timestamp_millis_opt(time?).single()?;
    let element = div()
        .id(ElementId::NamedInteger("message-time".into(), key))
        .flex_none()
        .px_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(short(time, Local::now()))
        .tip_text(full(time));
    Some(element.into_any_element())
}

/// "14:32" today, "21.10 14:32" on another day of this year, with the year
/// further back.
fn short(time: DateTime<Local>, now: DateTime<Local>) -> String {
    if time.date_naive() == now.date_naive() {
        time.format("%H:%M").to_string()
    } else if time.year() == now.year() {
        time.format("%d.%m %H:%M").to_string()
    } else {
        time.format("%d.%m.%Y %H:%M").to_string()
    }
}

/// "22.10.2026 14:32:05".
fn full(time: DateTime<Local>) -> String {
    time.format("%d.%m.%Y %H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Local, TimeZone as _};

    use super::{full, short};

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 5)
            .single()
            .expect("unambiguous local time")
    }

    #[test]
    fn short_time_grows_with_distance() {
        let now = at(2026, 10, 22, 18, 0);
        assert_eq!(short(at(2026, 10, 22, 14, 32), now), "14:32");
        assert_eq!(short(at(2026, 10, 21, 9, 5), now), "21.10 09:05");
        assert_eq!(short(at(2025, 12, 31, 23, 59), now), "31.12.2025 23:59");
    }

    #[test]
    fn full_time_has_seconds() {
        assert_eq!(full(at(2026, 10, 22, 14, 32)), "22.10.2026 14:32:05");
    }
}
