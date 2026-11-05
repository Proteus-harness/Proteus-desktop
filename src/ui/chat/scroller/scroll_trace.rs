//! Bounded, opt-in debug diagnostics. Contains geometry only, never chat data.
use std::sync::{
    LazyLock,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

static ENABLED: LazyLock<bool> =
    LazyLock::new(|| std::env::var("PROTEUS_SCROLL_TRACE").as_deref() == Ok("1"));
static START: LazyLock<Instant> = LazyLock::new(Instant::now);
static COUNT: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn record(value: impl FnOnce() -> serde_json::Value) {
    if !*ENABLED {
        return;
    }
    let index = COUNT.fetch_add(1, Ordering::Relaxed);
    if index >= 6000 {
        return;
    }
    eprintln!(
        "PROTEUS_SCROLL {}",
        serde_json::json!({
            "index": index, "elapsed_ms": START.elapsed().as_secs_f64()*1000., "state": value(),
        })
    );
}
