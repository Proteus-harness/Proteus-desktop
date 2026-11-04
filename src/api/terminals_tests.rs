use super::*;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{TcpListener, TcpStream};

fn request(socket: &mut TcpStream) -> (String, serde_json::Value) {
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(socket.try_clone().unwrap());
    let mut first = String::new();
    reader.read_line(&mut first).unwrap();
    let mut size = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty());
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            size = value.trim().parse().unwrap();
        }
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes).unwrap();
    (
        first,
        if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}
fn reply(socket: &mut TcpStream, status: u16, body: &str) {
    write!(socket,"HTTP/1.1 {status} Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
}

#[test]
fn terminal_requests_address_exact_session_uuid_without_model_commands() {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let api = Api::new(
        &format!("http://{}", server.local_addr().unwrap()),
        "synthetic",
    );
    let id = Uuid::from_u128(77);
    let bytes = b"\xff\x00\x1b[31m\xd0";
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = server.accept().unwrap();
        let (path, _) = request(&mut socket);
        let url = url::Url::parse(&format!(
            "http://local/{}",
            path.split_whitespace()
                .nth(1)
                .unwrap()
                .trim_start_matches('/')
        ))
        .unwrap();
        assert_eq!(url.path(), "/terminals/read");
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["session_dir"], "/exact/session");
        assert_eq!(query["id"], id.to_string());
        assert_eq!(query["cursor"], "4");
        assert_eq!(query["max_bytes"], "65536");
        let response = ToolTerminalResponse::Read(ToolTerminalSnapshot {
            start_cursor: 4,
            next_cursor: 4 + bytes.len() as u64,
            end_cursor: 4 + bytes.len() as u64,
            data_base64: base64::prelude::BASE64_STANDARD.encode(bytes),
            gap: false,
            finished: false,
            exit_code: None,
            tty: true,
            columns: 80,
            rows: 24,
        });
        reply(&mut socket, 200, &serde_json::to_string(&response).unwrap());
        for operation in ["input", "resize", "stop"] {
            let (mut socket, _) = server.accept().unwrap();
            let (path, body) = request(&mut socket);
            assert_eq!(path, format!("POST /terminals/{operation} HTTP/1.1\r\n"));
            assert_eq!(body["session_dir"], "/exact/session");
            assert_eq!(body["id"], id.to_string());
            if operation == "input" {
                assert_eq!(
                    base64::prelude::BASE64_STANDARD
                        .decode(body["data_base64"].as_str().unwrap())
                        .unwrap(),
                    b"synthetic-private\r"
                );
            }
            if operation == "resize" {
                assert_eq!(body["columns"], 101);
                assert_eq!(body["rows"], 31);
            }
            reply(&mut socket, 200, r#"{"type":"accepted"}"#);
        }
    });
    let session = Path::new("/exact/session");
    let read = api.terminal_read(session, id, Some(4)).unwrap();
    assert_eq!(
        base64::prelude::BASE64_STANDARD
            .decode(read.data_base64)
            .unwrap(),
        bytes
    );
    api.terminal_input(session, id, b"synthetic-private\r")
        .unwrap();
    api.terminal_resize(session, id, 101, 31).unwrap();
    api.terminal_stop(session, id).unwrap();
    worker.join().unwrap();
}

#[test]
fn unavailable_and_failed_input_are_not_accepted_or_retried() {
    for status in [0, 200, 400] {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(
            &format!("http://{}", server.local_addr().unwrap()),
            "synthetic",
        );
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            request(&mut socket);
            if status == 0 {
                drop(socket);
            } else {
                reply(
                    &mut socket,
                    status,
                    if status == 200 {
                        r#"{"type":"unavailable"}"#
                    } else {
                        r#"{"type":"accepted"}"#
                    },
                );
            }
            server.set_nonblocking(true).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            assert!(server.accept().is_err(), "input was retried");
        });
        let error = api
            .terminal_input(
                Path::new("/exact/session"),
                Uuid::from_u128(77),
                b"synthetic-private",
            )
            .unwrap_err();
        assert!(!format!("{error:#}").contains("synthetic-private"));
        worker.join().unwrap();
    }
}
