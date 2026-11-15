//! Exercise the actual Runtime producer and NDJSON transport, without a backend.

use super::*;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("proteus-navigation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn received(&self, count: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let rows: Vec<_> = std::fs::read_to_string(self.0.join("received.jsonl"))
                .unwrap_or_default()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect();
            if rows.len() >= count {
                assert_eq!(rows.len(), count);
                return rows;
            }
            assert!(Instant::now() < deadline, "extension fixture timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[::core::prelude::v1::test]
fn actual_scope_bootstrap_seeds_each_context_after_context_and_snapshot() {
    let scratch = Scratch::new();
    let installed = Box::leak(Box::new(Installed {
        dir: scratch.0.clone(),
        revision: 0,
        manifest: serde_json::from_value(json!({
            "protocol": PROTOCOL,
            "id": "navigation-fixture",
            "title": "Navigation fixture",
            "session_events": true,
            "surfaces": {"document": {}},
            "command": ["python3", "-u", "-c",
                "import sys\nwith open('received.jsonl', 'w') as out:\n for line in sys.stdin:\n  out.write(line)\n  out.flush()"]
        }))
        .unwrap(),
    }));
    let mut cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = cx.add_empty_window();
    let (runtime, signals, first, second) = window.update(|window, cx| {
        let a = cx.new(|cx| Workspace::test_empty(window, cx));
        let b = cx.new(|cx| Workspace::test_empty(window, cx));
        let host_a = Host::new(a.downgrade(), "navigation-fixture");
        let host_b = Host::new(b.downgrade(), "navigation-fixture");
        let first = host_a.scope();
        let second = host_b.scope();
        assert_ne!(first, second);
        let runtime = cx.new(|cx| Runtime::new(installed, host_a.clone(), window, cx));
        let signals = runtime.update(cx, |runtime, cx| {
            runtime.ensure_scope(host_a, window.window_handle(), cx);
            runtime.announce_scope(&first, cx);
            assert!(
                !runtime.scopes[&first].announced,
                "no process received that attempt"
            );
            let (process, signals) = Process::spawn(installed).unwrap();
            runtime.process = Some(process);
            runtime.announce_scope(&first, cx);
            assert!(runtime.scopes[&first].announced);
            // Existing context gets its ordinary metadata/snapshot refresh only.
            runtime.announce_scope(&first, cx);
            // Another context with the same session still needs its own navigation.
            runtime.ensure_scope(host_b, window.window_handle(), cx);
            signals
        });
        (runtime, signals, first, second)
    });
    let rows = scratch.received(7);
    assert_eq!(
        rows.iter()
            .map(|row| row["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "context",
            "session_snapshot",
            "event",
            "context",
            "session_snapshot",
            "context",
            "event"
        ]
    );
    for (row, expected) in [(&rows[2], &first), (&rows[6], &second)] {
        assert_eq!(row["context"], expected.as_str());
        assert_eq!(row["event"], "navigation_changed");
        assert!(
            row.get("document").is_none(),
            "an absent active document is not guessed"
        );
    }
    runtime.update(&mut cx, |runtime, _| runtime.process = None);
    drop(signals);
}
