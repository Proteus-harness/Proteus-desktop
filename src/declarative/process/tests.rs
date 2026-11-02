use super::*;
use crate::declarative::manifest::{Manifest, Surfaces};
use crate::declarative::protocol::HostEventKind;

fn next(signals: &Signals) -> Signal {
    signals
        .recv_blocking()
        .unwrap()
        .with_signal(|signal| signal)
}

fn fixture(script: &str) -> Installed {
    Installed {
        dir: std::env::temp_dir(),
        revision: 0,
        manifest: Manifest {
            protocol: super::super::protocol::PROTOCOL,
            required_features: Vec::new(),
            metadata: Default::default(),
            id: "transport-test".into(),
            title: "Transport test".into(),
            author: String::new(),
            description: String::new(),
            icon: None,
            command: vec!["python3".into(), "-u".into(), "-c".into(), script.into()],
            session_events: false,
            session_entries: true,
            surfaces: Surfaces::default(),
            dependencies: Default::default(),
            agent_instructions: Vec::new(),
        },
    }
}

fn budgets(count: usize) -> (Arc<Budget>, Arc<Budget>) {
    (
        Budget::new(count, INCOMING_BYTES),
        Budget::new(count, OUTGOING_BYTES),
    )
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "fixture timed out");
        thread::sleep(Duration::from_millis(2));
    }
}

fn child_gone(pid: u32) -> bool {
    // ESRCH after wait, including the transition through an exited zombie.
    unsafe { libc::kill(pid as libc::pid_t, 0) == -1 }
}

fn pid(process: &Process) -> u32 {
    process.state.child.lock().unwrap().as_ref().unwrap().id()
}

fn action(sequence: u64, bytes: usize) -> Outgoing {
    Outgoing::Action {
        sequence,
        view: "fixture".into(),
        action: "append".into(),
        value: Some(serde_json::Value::String("x".repeat(bytes))),
    }
}

/// Framing tests have no child or reader thread: arbitrary chunk boundaries
/// come from a real BufReader, while the normal transport budget stays active.
fn reader_state() -> State {
    let (messages, queued_messages) = async_channel::bounded(QUEUE_COUNT);
    let (writer, queued_writes) = async_channel::bounded(QUEUE_COUNT);
    let (failure, _errors) = async_channel::bounded(1);
    State {
        child: Mutex::new(None),
        closed: AtomicBool::new(false),
        incoming: Budget::new(QUEUE_COUNT, INCOMING_BYTES),
        outgoing: Budget::new(QUEUE_COUNT, OUTGOING_BYTES),
        messages,
        queued_messages,
        writer,
        queued_writes,
        failure,
    }
}

#[test]
fn framing_reassembles_utf8_json_across_single_byte_chunks() {
    let source = "{\"type\":\"clipboard\",\"text\":\"кофе ☕😀\\nготово\"}\n";
    let state = reader_state();
    let mut reader = BufReader::with_capacity(1, source.as_bytes());
    let (bytes, permit) = io::read_frame(&mut reader, &state).unwrap().unwrap();
    assert_eq!(state.incoming.usage(), (1, source.len()));
    assert_eq!(bytes, source.as_bytes()[..source.len() - 1]);
    assert!(matches!(
        protocol::parse(std::str::from_utf8(&bytes).unwrap()).unwrap(),
        Incoming::Clipboard { text } if text == "кофе ☕😀\nготово"
    ));
    drop((bytes, permit));
    assert_eq!(state.incoming.usage(), (0, 0));
    assert!(io::read_frame(&mut reader, &state).unwrap().is_none());
    assert_eq!(state.incoming.usage(), (0, 0));
}

#[test]
fn framing_accepts_a_complete_final_json_message_without_line_feed() {
    let source = br#"{"type":"clipboard","text":"last message"}"#;
    let state = reader_state();
    let mut reader = BufReader::with_capacity(3, source.as_slice());
    let (bytes, permit) = io::read_frame(&mut reader, &state).unwrap().unwrap();
    assert_eq!(state.incoming.usage(), (1, source.len()));
    assert_eq!(bytes, source);
    assert!(matches!(
        protocol::parse(std::str::from_utf8(&bytes).unwrap()).unwrap(),
        Incoming::Clipboard { text } if text == "last message"
    ));
    drop((bytes, permit));
    assert!(io::read_frame(&mut reader, &state).unwrap().is_none());
    assert_eq!(state.incoming.usage(), (0, 0));
}

#[test]
fn framing_accepts_exact_byte_limit_plus_newline_and_releases_rejected_frame() {
    let state = reader_state();
    for extra in [0, 1] {
        let mut source = br#"{"type":"clipboard","text":""#.to_vec();
        source.resize(io::MAX_FRAME + extra - 2, b'x');
        source.extend_from_slice(b"\"}\n");
        assert_eq!(source.len(), io::MAX_FRAME + extra + 1);
        // Odd-sized chunks exercise the byte boundary independently from
        // both the frame limit and BufReader's ordinary power-of-two size.
        let mut reader = BufReader::with_capacity(8191, source.as_slice());
        let result = io::read_frame(&mut reader, &state);
        if extra == 0 {
            let (bytes, permit) = result.unwrap().unwrap();
            assert_eq!(bytes.len(), io::MAX_FRAME);
            assert_eq!(state.incoming.usage(), (1, io::MAX_FRAME + 1));
            assert!(bytes.ends_with(b"\"}"));
            drop((bytes, permit));
            assert!(io::read_frame(&mut reader, &state).unwrap().is_none());
        } else {
            assert!(
                result.is_err(),
                "one excess content byte must fail before LF"
            );
        }
        assert_eq!(state.incoming.usage(), (0, 0));
    }
}

#[test]
fn incoming_backpressure_preserves_every_message_and_current_delivery() {
    let limits = budgets(2);
    let (process, signals) = Process::spawn_with_budgets(&fixture(
            "import json\nfor i in range(20): print(json.dumps({'type':'clipboard','text':str(i)}), flush=True)"
        ), limits.clone()).unwrap();
    wait_for(|| limits.0.usage().0 == 2 && signals.incoming.len() == 2);
    let held = signals.recv_blocking().unwrap();
    assert_eq!(
        limits.0.usage().0,
        2,
        "receiving must not release admission"
    );
    held.with_signal(|signal| {
        assert!(matches!(signal, Signal::Message(Incoming::Clipboard { text }) if text == "0"));
        assert_eq!(limits.0.usage().0, 2, "handler owns the delivery permit");
    });
    for index in 1..20 {
        assert!(
            matches!(next(&signals), Signal::Message(Incoming::Clipboard { text }) if text == index.to_string())
        );
    }
    assert!(matches!(next(&signals), Signal::Exited(_)));
    drop(process);
    assert_eq!(limits.0.usage(), (0, 0));
}

#[test]
fn incoming_budget_is_shared_across_processes_and_shutdown_unblocks_reader() {
    let limits = budgets(1);
    let script = "import json,time\nfor i in range(100): print(json.dumps({'type':'clipboard','text':str(i)}), flush=True)\ntime.sleep(30)";
    let (first, first_signals) =
        Process::spawn_with_budgets(&fixture(script), limits.clone()).unwrap();
    wait_for(|| first_signals.incoming.len() == 1);
    let (second, second_signals) =
        Process::spawn_with_budgets(&fixture(script), limits.clone()).unwrap();
    let first_pid = pid(&first);
    let second_pid = pid(&second);
    let second_state = Arc::downgrade(&second.state);
    wait_for(|| limits.0.waiters() == 2);
    assert_eq!(limits.0.usage().0, 1);
    assert!(second_signals.incoming.is_empty());
    drop((second, second_signals));
    wait_for(|| child_gone(second_pid));
    wait_for(|| second_state.upgrade().is_none());
    // Even while another child's permit is retained, a stopped reader
    // must leave the shared condvar and release its process resources.
    assert_eq!(limits.0.usage().0, 1);
    drop(first);
    wait_for(|| child_gone(first_pid));
    wait_for(|| limits.0.usage() == (0, 0));
}

#[test]
fn blocked_writer_is_in_budget_and_overflow_is_a_visible_terminal_failure() {
    let limits = budgets(2);
    let (process, signals) =
        Process::spawn_with_budgets(&fixture("import time; time.sleep(30)"), limits.clone())
            .unwrap();
    let child = pid(&process);
    let message = action(1, 1024 * 1024);
    process.send(&message).unwrap();
    process.send(&message).unwrap();
    assert_eq!(limits.1.usage().0, 2, "currently blocked write counts too");
    let used = limits.1.usage().1;
    assert!(used >= 2 * 1024 * 1024 && used < 3 * 1024 * 1024);
    let started = std::time::Instant::now();
    assert!(process.send(&message).is_err());
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "UI must not wait for the pipe"
    );
    assert!(matches!(next(&signals), Signal::Exited(text) if text.contains("очередь")));
    assert!(process.send(&message).is_err());
    wait_for(|| child_gone(child));
    wait_for(|| limits.1.usage() == (0, 0));
}

#[test]
fn writer_byte_budget_applies_across_children_before_encoding() {
    let outgoing = Budget::new(128, 2 * 1024 * 1024);
    let incoming = Budget::new(128, INCOMING_BYTES);
    let limits = (incoming, outgoing.clone());
    let script = "import time; time.sleep(30)";
    let (first, first_signals) =
        Process::spawn_with_budgets(&fixture(script), limits.clone()).unwrap();
    let (second, second_signals) = Process::spawn_with_budgets(&fixture(script), limits).unwrap();
    let message = action(1, 1024 * 1024);
    first.send(&message).unwrap();
    assert!(second.send(&message).is_err());
    assert!(matches!(next(&second_signals), Signal::Exited(_)));
    assert_eq!(outgoing.usage().0, 1);
    drop((first, first_signals, second, second_signals));
    wait_for(|| outgoing.usage() == (0, 0));
}

#[test]
fn admitted_actions_arrive_in_the_original_order() {
    let script = "import sys,json\nfor line in sys.stdin:\n m=json.loads(line)\n print(json.dumps({'type':'clipboard','text':str(m['sequence'])}), flush=True)";
    let (process, signals) = Process::spawn_with_budgets(&fixture(script), budgets(128)).unwrap();
    for index in 0..32 {
        process.send(&action(index, 4)).unwrap();
    }
    for index in 0..32 {
        assert!(
            matches!(next(&signals), Signal::Message(Incoming::Clipboard { text }) if text == index.to_string())
        );
    }
}

#[test]
fn write_failure_is_reported_even_with_stdout_backpressured() {
    let script = "import os,json,time\nos.close(0)\nfor i in range(100): print(json.dumps({'type':'clipboard','text':str(i)}),flush=True)\ntime.sleep(30)";
    let limits = budgets(1);
    let (process, signals) = Process::spawn_with_budgets(&fixture(script), limits.clone()).unwrap();
    let child = pid(&process);
    wait_for(|| signals.incoming.len() == 1);
    process.send(&action(1, 64 * 1024)).unwrap();
    wait_for(|| process.state.closed.load(Ordering::Acquire));
    assert!(matches!(next(&signals), Signal::Exited(text) if text.contains("записи")));
    wait_for(|| child_gone(child));
    wait_for(|| limits.0.usage() == (0, 0) && limits.1.usage() == (0, 0));
}

#[test]
fn size_count_matches_escaped_utf8_json_and_rejects_oversized_frame() {
    let message = Outgoing::Action {
        sequence: 1,
        view: "файл\n\"".into(),
        action: "\\".into(),
        value: Some(serde_json::json!({"key":"\u{0}\nя😀"})),
    };
    assert_eq!(
        io::encoded_size(&message).unwrap(),
        serde_json::to_vec(&message).unwrap().len() + 1
    );
    let message = action(2, io::MAX_FRAME);
    assert!(io::encoded_size(&message).is_err());
}

#[test]
fn oversized_stdout_fails_without_waiting_for_newline_or_echoing_payload() {
    let (process, signals) = Process::spawn_with_budgets(&fixture(
            "import sys,time\nsys.stdout.write('secret fixture'+ 'x'*(16*1024*1024));sys.stdout.flush();time.sleep(30)"
        ), budgets(2)).unwrap();
    let child = pid(&process);
    assert!(
        matches!(next(&signals), Signal::Exited(text) if text.contains("16 МиБ") && !text.contains("secret fixture"))
    );
    wait_for(|| child_gone(child));
}

#[test]
fn stderr_truncates_long_lines_and_still_drains_later_lines() {
    let input = format!("{}\nlast\n", "x".repeat(io::MAX_LOG_LINE * 4));
    let mut lines = Vec::new();
    io::read_log(std::io::Cursor::new(input), |line| lines.push(line));
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0].len(), io::MAX_LOG_LINE);
    assert_eq!(lines[1], "last");
}

#[test]
fn exchanges_lines_and_reports_the_exit() {
    let script = r#"read hello; printf '\377\n'; echo '{"type":"view","view":"panel","root":{"type":"text","text":"ok"}}'; echo 'junk'; echo 'bye' >&2"#;
    let extension = Installed {
        dir: std::env::temp_dir(),
        revision: 0,
        manifest: Manifest {
            protocol: super::super::protocol::PROTOCOL,
            required_features: Vec::new(),
            metadata: Default::default(),
            id: "test".into(),
            title: "Test".into(),
            author: String::new(),
            description: String::new(),
            icon: None,
            command: vec!["sh".into(), "-c".into(), script.into()],
            session_events: false,
            session_entries: true,
            surfaces: Surfaces::default(),
            dependencies: Default::default(),
            agent_instructions: Vec::new(),
        },
    };
    let (process, signals) = Process::spawn(&extension).unwrap();
    process
        .send(&Outgoing::Event {
            context: "fixture".into(),
            event: HostEventKind::RunSettled,
            document: None,
            session_dir: None,
        })
        .unwrap();
    // A line that is not UTF-8 is a bad message, not the end of output.
    assert!(matches!(next(&signals), Signal::Invalid(_)));
    let view = next(&signals);
    assert!(
        matches!(view, Signal::Message(Incoming::View { .. })),
        "{view:?}"
    );
    assert!(matches!(next(&signals), Signal::Invalid(_)));
    let Signal::Exited(text) = next(&signals) else {
        panic!("exit expected");
    };
    assert!(text.ends_with("bye"), "{text}");
}
