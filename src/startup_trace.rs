//! Opt-in timings, without session paths, request bodies or credentials.
use std::{
    collections::HashSet,
    sync::{LazyLock, Mutex},
    time::Instant,
};

static TRACE: LazyLock<Option<Instant>> = LazyLock::new(|| {
    (std::env::var("PROTEUS_STARTUP_TRACE").as_deref() == Ok("1")).then(Instant::now)
});
static SEEN: LazyLock<Mutex<HashSet<&'static str>>> = LazyLock::new(Mutex::default);

pub fn mark(stage: &str) {
    if let Some(start) = *TRACE {
        eprintln!(
            "PROTEUS_STARTUP {}",
            serde_json::json!({"stage":stage,"elapsed_ms":start.elapsed().as_secs_f64()*1000.0})
        );
    }
}

pub fn first(stage: &'static str) {
    if TRACE.is_some() && SEEN.lock().unwrap_or_else(|e| e.into_inner()).insert(stage) {
        mark(stage);
    }
}

pub fn history_window(start: usize, rows: usize, bytes: impl FnOnce() -> usize, live_rows: usize) {
    if let Some(record) = history_window_record(TRACE.is_some(), start, rows, bytes, live_rows) {
        eprintln!("PROTEUS_HISTORY {record}");
    }
}

fn history_window_record(
    enabled: bool,
    start: usize,
    rows: usize,
    bytes: impl FnOnce() -> usize,
    live_rows: usize,
) -> Option<serde_json::Value> {
    // Payload accounting walks nested tool arguments/metadata. Do not pay
    // that diagnostic cost when the trace is disabled.
    enabled.then(|| {
        serde_json::json!({
            "start": start, "rows": rows, "payload_bytes": bytes(), "live_rows": live_rows,
        })
    })
}

pub fn span(stage: &str) -> Span<'_> {
    Span {
        stage,
        start: TRACE.map(|_| Instant::now()),
    }
}

pub struct Span<'a> {
    stage: &'a str,
    start: Option<Instant>,
}
impl Drop for Span<'_> {
    fn drop(&mut self) {
        if let (Some(start), Some(trace)) = (self.start, *TRACE) {
            eprintln!(
                "PROTEUS_STARTUP {}",
                serde_json::json!({"stage":self.stage,"duration_ms":start.elapsed().as_secs_f64()*1000.0,"elapsed_ms":trace.elapsed().as_secs_f64()*1000.0})
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::history_window_record;
    use std::cell::Cell;

    #[test]
    fn disabled_history_trace_does_not_measure_payload() {
        assert!(
            history_window_record(false, 150, 50, || panic!("payload must stay unvisited"), 50)
                .is_none()
        );
    }

    #[test]
    fn enabled_history_trace_measures_once_and_preserves_its_record() {
        let calls = Cell::new(0);
        let record = history_window_record(
            true,
            150,
            50,
            || {
                calls.set(calls.get() + 1);
                123_456
            },
            17,
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(
            record,
            Some(serde_json::json!({
                "start": 150, "rows": 50, "payload_bytes": 123_456, "live_rows": 17,
            }))
        );
    }

    #[test]
    fn enabled_history_trace_accepts_a_once_only_measurement_without_caching() {
        for expected in [0, 4096, usize::MAX] {
            let measurements = vec![expected];
            let record =
                history_window_record(true, 0, 1, move || measurements.into_iter().sum(), 1)
                    .unwrap();
            assert_eq!(record["payload_bytes"], serde_json::json!(expected));
        }
    }
}
