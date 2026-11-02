use super::*;
use std::sync::mpsc;

const TIMEOUT: Duration = Duration::from_secs(2);

#[test]
fn delayed_write_keeps_only_latest_snapshot_and_quit_flushes_it() {
    let (started, writes) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let (writer, _) = Writer::new(move |value: &usize| {
        started.send(*value).unwrap();
        if *value == 1 {
            blocked.recv_timeout(TIMEOUT).unwrap();
        }
        Ok(())
    })
    .unwrap();
    writer.submit(Arc::new(1)).unwrap();
    assert_eq!(writes.recv_timeout(TIMEOUT).unwrap(), 1);
    let mut previous = std::sync::Weak::new();
    for value in 2..=10_000 {
        let snapshot = Arc::new(value);
        writer.submit(snapshot.clone()).unwrap();
        // Replaced values are not retained in a per-edit queue.
        assert!(previous.upgrade().is_none());
        previous = Arc::downgrade(&snapshot);
    }
    assert!(writes.try_recv().is_err());
    let shutdown = writer.shutdown();
    assert!(writer.submit(Arc::new(10_001)).is_err());
    release.send(()).unwrap();
    assert!(shutdown.wait(TIMEOUT));
    assert_eq!(writes.recv_timeout(TIMEOUT).unwrap(), 10_000);
    assert!(writes.try_recv().is_err());
    let completion = writer.completion().unwrap();
    assert_eq!(completion.revision, 10_000);
    assert!(completion.result.is_ok());
}

#[test]
fn slow_filesystem_cannot_hold_quit_indefinitely() {
    let (started, wait) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let (writer, _) = Writer::new(move |_: &usize| {
        started.send(()).unwrap();
        blocked.recv_timeout(TIMEOUT).unwrap();
        Ok(())
    })
    .unwrap();
    writer.submit(Arc::new(1)).unwrap();
    wait.recv_timeout(TIMEOUT).unwrap();
    assert!(!writer.shutdown().wait(Duration::ZERO));
    release.send(()).unwrap();
    assert!(writer.shutdown().wait(TIMEOUT));
}

#[test]
fn stale_success_or_failure_cannot_change_latest_error() {
    for result in [Ok(()), Err("old failure".into())] {
        let completion = Completion {
            revision: 1,
            result,
        };
        assert_eq!(completion.problem_for(2), None);
    }
    assert_eq!(
        Completion {
            revision: 2,
            result: Err("disk full".into())
        }
        .problem_for(2),
        Some(Some("Не сохранено: disk full".into()))
    );
    assert_eq!(
        Completion {
            revision: 3,
            result: Ok(())
        }
        .problem_for(3),
        Some(None)
    );
}

#[test]
fn failed_write_does_not_stop_later_success_and_notifications_are_bounded() {
    let (writer, notices) = Writer::new(|value: &usize| {
        if *value == 1 {
            Err("disk full".into())
        } else {
            Ok(())
        }
    })
    .unwrap();
    writer.submit(Arc::new(1)).unwrap();
    notices.recv_blocking().unwrap();
    assert_eq!(writer.completion().unwrap().result, Err("disk full".into()));
    writer.submit(Arc::new(2)).unwrap();
    assert!(writer.shutdown().wait(TIMEOUT));
    assert_eq!(notices.len(), 1);
    let completion = writer.completion().unwrap();
    assert_eq!(completion.revision, 2);
    assert!(completion.result.is_ok());
}

#[test]
fn dropping_writer_drains_pending_value_and_exits() {
    let (written, receiver) = mpsc::channel();
    let (writer, notices) = Writer::new(move |value: &usize| {
        written.send(*value).unwrap();
        Ok(())
    })
    .unwrap();
    writer.submit(Arc::new(42)).unwrap();
    drop(writer);
    assert_eq!(receiver.recv_timeout(TIMEOUT).unwrap(), 42);
    assert!(receiver.recv_timeout(TIMEOUT).is_err());
    while notices.recv_blocking().is_ok() {}
    assert!(notices.is_closed());
}

#[test]
fn atomic_preference_save_preserves_old_file_on_failure_and_retries() {
    use crate::preferences::{self, Preferences};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build/verification/preferences-writer-tests")
        .join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("preferences.json");
    let before = Preferences::default();
    preferences::save_at(&before, &path).unwrap();
    let old_file = std::fs::read(&path).unwrap();
    let draft = path.with_extension("json.tmp");
    std::fs::create_dir(&draft).unwrap();
    let target = path.clone();
    let (writer, notices) = Writer::new(move |value| {
        preferences::save_at(value, &target).map_err(|error| format!("{error:#}"))
    })
    .unwrap();
    let mut after = before;
    after.view.chat_width = 1000;
    writer.submit(Arc::new(after.clone())).unwrap();
    notices.recv_blocking().unwrap();
    assert!(writer.completion().unwrap().result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), old_file);
    std::fs::remove_dir(&draft).unwrap();
    writer.submit(Arc::new(after.clone())).unwrap();
    assert!(writer.shutdown().wait(TIMEOUT));
    assert!(writer.completion().unwrap().result.is_ok());
    let saved: Preferences = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved, after);
    assert!(!draft.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
