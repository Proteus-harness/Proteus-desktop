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
