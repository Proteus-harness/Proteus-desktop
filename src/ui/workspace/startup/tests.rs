use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::{Value, json};

use super::*;
use crate::tabs::{Edge, PaneId};

#[test]
fn startup_resumes_split_chats_without_resuming_hidden_tabs_or_reordering_them() {
    let server = Server::start(&["/sessions/right", "/sessions/hidden", "/sessions/left"]);
    let mut saved = TabLayout::default();
    for dir in [
        "/sessions/hidden",
        "/sessions/deleted",
        "/sessions/left",
        "/sessions/right",
    ] {
        saved.open(&tabs::chat(Path::new(dir)), |_| false);
    }
    saved.split("chat:/sessions/right", PaneId(0), Edge::Right);
    saved.activate(PaneId(0), "chat:/sessions/left");
    assert_eq!(saved.validate(), Ok(()));
    assert_eq!(
        saved.visible().collect::<Vec<_>>(),
        ["chat:/sessions/left", "chat:/sessions/right"]
    );

    let boot = restore(server.api(), None, saved).unwrap();

    assert_eq!(boot.resumed, paths(&["/sessions/left", "/sessions/right"]));
    assert_eq!(
        boot.restored,
        paths(&["/sessions/hidden", "/sessions/left", "/sessions/right"])
    );
    assert!(boot.startup.is_none());
    assert_eq!(
        server.calls(),
        [
            ("/sessions".into(), Value::Null),
            ("/resume".into(), json!("/sessions/left")),
            ("/resume".into(), json!("/sessions/right")),
        ]
    );
}

#[test]
fn focused_document_resumes_only_the_hidden_command_target() {
    let server = Server::start(&["/sessions/other", "/sessions/target"]);
    let mut saved = TabLayout::default();
    saved.open("chat:/sessions/target", |_| false);
    saved.open("chat:/sessions/other", |_| false);
    saved.open("files:src/main.rs", |_| false);
    assert_eq!(saved.visible().collect::<Vec<_>>(), ["files:src/main.rs"]);
    assert_eq!(
        saved.active_chat(None).as_deref(),
        Some("chat:/sessions/target")
    );

    let boot = restore(server.api(), None, saved).unwrap();

    assert_eq!(boot.resumed, paths(&["/sessions/target"]));
    assert_eq!(
        boot.restored,
        paths(&["/sessions/target", "/sessions/other"])
    );
    assert!(boot.startup.is_none());
    assert_eq!(
        server.calls(),
        [
            ("/sessions".into(), Value::Null),
            ("/resume".into(), json!("/sessions/target")),
        ]
    );
}

fn paths(dirs: &[&str]) -> Vec<PathBuf> {
    dirs.iter().map(PathBuf::from).collect()
}

struct Server {
    address: SocketAddr,
    calls: Arc<Mutex<Vec<(String, Value)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Server {
    fn start(dirs: &[&str]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let catalog: Vec<_> = dirs
            .iter()
            .map(|dir| {
                AppSessionSummary::new(
                    PathBuf::from(dir),
                    proteus_contracts::domain::new_session_id(),
                    PathBuf::from("/workspace"),
                    0,
                    None,
                    None,
                )
            })
            .collect();
        let worker = {
            let calls = calls.clone();
            let stop = stop.clone();
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => respond(stream, &catalog, &calls),
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("fixture listener: {error}"),
                    }
                }
            })
        };
        Self {
            address,
            calls,
            stop,
            worker: Some(worker),
        }
    }

    fn api(&self) -> Api {
        Api::new(&format!("http://{}", self.address), "startup-test")
    }

    fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let result = self.worker.take().unwrap().join();
        if !thread::panicking() {
            result.unwrap();
        }
    }
}

fn respond(stream: TcpStream, catalog: &[AppSessionSummary], calls: &Mutex<Vec<(String, Value)>>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let path = line.split_whitespace().nth(1).unwrap().to_owned();
    let mut length = 0;
    loop {
        line.clear();
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "incomplete HTTP headers"
        );
        if line == "\r\n" {
            break;
        }
        if let Some((header, value)) = line.split_once(':') {
            if header.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap();
            }
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).unwrap();
    let body: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    let dir = body.get("session_dir").cloned().unwrap_or(Value::Null);
    calls.lock().unwrap().push((path.clone(), dir.clone()));
    let response = match path.as_str() {
        "/sessions" => serde_json::to_value(catalog).unwrap(),
        "/resume" => json!({
            "type": "response", "id": body["id"], "ok": true, "error": null,
            "output": {"session_dir": dir, "activity": {
                "status": "idle", "running_runs": 0, "running_run_ids": [],
                "pending_approvals": 0, "pending_user_inputs": 0
            }}
        }),
        other => panic!("unexpected startup endpoint: {other}"),
    };
    let bytes = serde_json::to_vec(&response).unwrap();
    let stream = reader.get_mut();
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len()).unwrap();
    stream.write_all(&bytes).unwrap();
}
