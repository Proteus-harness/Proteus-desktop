use proteus_codex_client::{Client, Error, Event, Launch, Limits, RpcError};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn launch(limits: Limits) -> Launch {
    Launch {
        program: PathBuf::from("python3"),
        args: vec![
            "-u".into(),
            format!("{}/tests/peer.py", env!("CARGO_MANIFEST_DIR")),
        ],
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        limits,
    }
}

fn client() -> Client {
    Client::spawn(launch(Limits::default())).unwrap()
}

fn wait_closed(client: &Client) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while client.is_connected() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!client.is_connected());
}

#[test]
fn initializes_and_routes_concurrent_out_of_order_replies() {
    let client = client();
    assert!(client.take_events().is_some());
    assert!(client.clone().take_events().is_none());
    let other = client.clone();
    let first = thread::spawn(move || other.request("reverse", json!("first")).unwrap());
    assert_eq!(
        client.request("reverse", json!("second")).unwrap(),
        json!("second")
    );
    assert_eq!(first.join().unwrap(), json!("first"));
}

#[test]
fn cancellation_proceeds_with_approval_held_and_opaque_ids_round_trip() {
    for id in [json!("approval/string-7"), json!(-4321)] {
        let client = client();
        let events = client.take_events().unwrap();
        let other = client.clone();
        let server_id = id.clone();
        let pending =
            thread::spawn(move || other.request("approval", json!({"serverId": server_id})));
        assert!(
            matches!(events.recv_blocking().unwrap(), Event::Notification { method, .. } if method == "turn/started")
        );
        assert!(
            matches!(events.recv_blocking().unwrap(), Event::Request { id: actual, .. } if actual == id)
        );
        assert_eq!(
            client.request("turn/interrupt", json!({})).unwrap(),
            json!({"interrupted": true})
        );
        client
            .respond(id, Ok(json!({"decision": "decline"})))
            .unwrap();
        assert_eq!(
            pending.join().unwrap().unwrap(),
            json!({"decision": "decline"})
        );
    }
}

#[test]
fn structured_rpc_errors_and_server_error_response() {
    let client = client();
    assert_eq!(
        client.request("remote_error", Value::Null),
        Err(Error::Remote(RpcError {
            code: -123,
            message: "synthetic error".into(),
            data: Some(json!({"retry": false})),
        }))
    );
    let events = client.take_events().unwrap();
    let other = client.clone();
    let pending = thread::spawn(move || other.request("approval", json!({"serverId": 10})));
    events.recv_blocking().unwrap();
    events.recv_blocking().unwrap();
    client
        .respond(
            json!(10),
            Err(RpcError {
                code: -32601,
                message: "unsupported".into(),
                data: None,
            }),
        )
        .unwrap();
    assert_eq!(pending.join().unwrap().unwrap()["code"], -32601);
}

#[test]
fn eof_fails_all_pending_and_closes_events() {
    let client = client();
    let events = client.take_events().unwrap();
    let other = client.clone();
    let pending = thread::spawn(move || other.request("timeout", Value::Null));
    assert!(client.request("eof", Value::Null).is_err());
    assert!(pending.join().unwrap().is_err());
    wait_closed(&client);
    while events.recv_blocking().is_ok() {}
    assert!(events.is_closed());
}

#[test]
fn timeout_does_not_replay_or_misroute_late_reply() {
    let client = client();
    assert_eq!(
        client.request_timeout("late", Value::Null, Duration::from_millis(5)),
        Err(Error::TimedOut)
    );
    assert_eq!(
        client.request("echo", json!("fresh")).unwrap(),
        json!("fresh")
    );
    assert!(client.is_connected());
}

#[test]
fn oversized_and_malformed_frames_disconnect_without_echoing_body() {
    for method in ["oversize", "malformed"] {
        let client = Client::spawn(launch(Limits {
            max_line_bytes: 1024,
            ..Limits::default()
        }))
        .unwrap();
        let error = client.request(method, Value::Null).unwrap_err();
        assert!(!error.to_string().contains("secret fixture"));
        wait_closed(&client);
    }
}

#[test]
fn incoming_count_and_byte_overflow_close_instead_of_dropping_events() {
    for limits in [
        Limits {
            queue_capacity: 2,
            ..Limits::default()
        },
        Limits {
            max_queued_bytes: 1000,
            ..Limits::default()
        },
    ] {
        let client = Client::spawn(launch(limits)).unwrap();
        let events = client.take_events().unwrap();
        assert!(client.request("flood", Value::Null).is_err());
        wait_closed(&client);
        while events.recv_blocking().is_ok() {}
        assert!(events.is_closed());
    }
}

#[test]
fn replies_share_the_incoming_byte_budget_with_queued_notifications() {
    let client = Client::spawn(launch(Limits {
        max_queued_bytes: 400,
        ..Limits::default()
    }))
    .unwrap();
    let _events = client.take_events().unwrap();
    assert_eq!(
        client.request("fill_then_reply", Value::Null),
        Err(Error::Overloaded)
    );
    wait_closed(&client);
}

#[test]
fn write_failure_releases_pending_requests() {
    let client = client();
    let events = client.take_events().unwrap();
    client.request("close_stdin", Value::Null).unwrap();
    assert!(
        matches!(events.recv_blocking().unwrap(), Event::Notification { method, .. } if method == "stdin_closed")
    );
    assert!(client.request("echo", json!("after close")).is_err());
    wait_closed(&client);
}

#[test]
fn outbound_queue_and_pending_count_are_bounded() {
    let client = Client::spawn(launch(Limits {
        max_queued_bytes: 65536,
        max_line_bytes: 65536,
        queue_capacity: 1,
        max_pending_requests: 1,
        ..Limits::default()
    }))
    .unwrap();
    client.request("stop_reading", Value::Null).unwrap();
    let mut overloaded = false;
    for _ in 0..100 {
        if client.notify("large", json!("x".repeat(32768))) == Err(Error::Overloaded) {
            overloaded = true;
            break;
        }
    }
    assert!(overloaded);
    // shutdown unblocks the writer even when the peer stopped reading stdin.
    client.shutdown();
    assert!(!client.is_connected());

    let client = Client::spawn(launch(Limits {
        max_pending_requests: 1,
        ..Limits::default()
    }))
    .unwrap();
    let events = client.take_events().unwrap();
    let other = client.clone();
    let pending = thread::spawn(move || {
        other.request_timeout("timeout", Value::Null, Duration::from_millis(100))
    });
    assert!(
        matches!(events.recv_blocking().unwrap(), Event::Notification { method, .. } if method == "holding")
    );
    assert_eq!(client.request("echo", Value::Null), Err(Error::Overloaded));
    assert_eq!(pending.join().unwrap(), Err(Error::TimedOut));
}

#[test]
fn dropping_last_client_kills_and_reaps_only_its_child() {
    let client = client();
    let pid = client
        .request("pid", Value::Null)
        .unwrap()
        .as_u64()
        .unwrap();
    let copy = client.clone();
    drop(client);
    assert!(PathBuf::from(format!("/proc/{pid}")).exists());
    drop(copy);
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
}
