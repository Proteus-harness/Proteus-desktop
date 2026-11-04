use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use proteus_contracts::app_protocol::AppSessionSummary;
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
        paths(&[
            "/sessions/hidden",
            "/sessions/deleted",
            "/sessions/left",
            "/sessions/right"
        ])
    );
    assert!(boot.startup.is_none());
    assert_eq!(
        boot.projects.get(Path::new("/sessions/left")),
        Some(&PathBuf::from("/workspace"))
    );
    assert_eq!(
        server.calls(),
        [
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
    saved.activate(PaneId(0), "chat:/sessions/target");
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
        [("/resume".into(), json!("/sessions/target")),]
    );
}

#[test]
fn initial_launch_uses_suggested_session_without_reading_the_catalog() {
    let server = Server::start(&["/sessions/suggested", "/sessions/unrelated"]);
    let boot = restore(server.api(), None, TabLayout::default()).unwrap();
    assert_eq!(boot.startup, Some(PathBuf::from("/sessions/suggested")));
    assert!(boot.sessions.is_empty());
    assert_eq!(
        server.calls(),
        [
            ("/bootstrap".into(), Value::Null),
            ("/resume".into(), json!("/sessions/suggested")),
        ]
    );
}

#[test]
fn restored_document_knows_command_targets_workspace_before_catalog_discovery() {
    let server = Server::start_in(&[("/sessions/a", "/project-a"), ("/sessions/b", "/project-b")]);
    let mut saved = TabLayout::default();
    saved.open("chat:/sessions/a", |_| false);
    saved.open("chat:/sessions/b", |_| false);
    saved.open("files:src/main.rs", |_| false);
    let boot = restore(server.api(), None, saved).unwrap();
    assert!(boot.sessions.is_empty());
    assert_eq!(
        boot.projects.get(Path::new("/sessions/b")),
        Some(&PathBuf::from("/project-b"))
    );
    assert_eq!(boot.resumed, paths(&["/sessions/b"]));
    assert_eq!(server.calls(), [("/resume".into(), json!("/sessions/b"))]);
}

#[test]
fn codex_only_restore_keeps_hidden_core_tabs_without_starting_core_or_reading_catalog() {
    let mut saved = TabLayout::default();
    saved.open("chat:/sessions/hidden-core", |_| false);
    saved.open("codex:opaque:hidden", |_| false);
    saved.open("codex:opaque:target", |_| false);
    saved.open("files:src/main.rs", |_| false);
    let required = required_chats(&saved);
    assert_eq!(required, [ChatKey::Codex("opaque:target".into())]);
    let options = codex_options();
    let boot = boot_with(
        &options,
        saved,
        |_| panic!("Codex restore must not launch Core"),
        |_, _| Ok(codex_peer("opaque:target", false)),
    )
    .unwrap();
    assert!(boot.api.is_none());
    assert!(boot.backend.is_none());
    assert!(boot.streams.is_empty());
    assert_eq!(boot.restored, paths(&["/sessions/hidden-core"]));
    assert_eq!(boot.codex_opened.len(), 1);
    assert_eq!(boot.codex_opened[0].metadata.id, "opaque:target");
    assert!(boot.codex_opened[0].history.data.is_empty());
}

#[test]
fn fresh_codex_launch_creates_own_thread_without_choosing_other_clients_history() {
    let boot = boot_with(
        &codex_options(),
        TabLayout::default(),
        |_| panic!("Fresh Codex launch must not start Core"),
        |_, _| Ok(codex_peer("", false)),
    )
    .unwrap();
    assert!(boot.api.is_none());
    assert_eq!(boot.codex_opened[0].metadata.id, "opaque:own-new");
    assert!(boot.codex.is_some());
}

#[test]
fn failed_codex_thread_is_local_and_does_not_create_a_replacement_thread() {
    let mut saved = TabLayout::default();
    saved.open("codex:opaque:missing", |_| false);
    let boot = boot_with(
        &codex_options(),
        saved,
        |_| panic!("Missing Codex thread must not start Core"),
        |_, _| Ok(codex_peer("opaque:missing", true)),
    )
    .unwrap();
    assert!(boot.codex.is_some());
    assert!(boot.codex_opened.is_empty());
    assert_eq!(boot.codex_failed.len(), 1);
    assert_eq!(boot.codex_failed[0].0, "codex:opaque:missing");
    assert!(boot.codex_failed[0].1.contains("synthetic missing thread"));
}

#[test]
fn missing_codex_thread_preserves_healthy_split_restoration() {
    let mut saved = TabLayout::default();
    saved.open("codex:opaque:healthy", |_| false);
    saved.open("codex:opaque:missing", |_| false);
    saved.split("codex:opaque:missing", PaneId(0), Edge::Right);
    let boot = boot_with(
        &codex_options(),
        saved,
        |_| panic!("Codex restore must not launch Core"),
        |_, _| Ok(codex_peer_many(&["opaque:healthy", "opaque:missing"], true)),
    )
    .unwrap();
    assert_eq!(boot.codex_opened.len(), 1);
    assert_eq!(boot.codex_opened[0].metadata.id, "opaque:healthy");
    assert_eq!(boot.codex_failed.len(), 1);
}

#[test]
fn lost_codex_transport_still_fails_restoration() {
    let mut saved = TabLayout::default();
    saved.open("codex:opaque:disconnected", |_| false);
    let result = boot_with(
        &codex_options(),
        saved,
        |_| panic!("Codex restore must not launch Core"),
        |_, _| Ok(codex_peer("opaque:disconnected", false)),
    );
    assert!(result.is_err());
}

#[test]
fn visible_driver_keys_determine_connections_independently_of_preference() {
    assert!(needs_core(AgentKind::Proteus, &[]));
    assert!(!needs_core(AgentKind::Codex, &[]));
    assert!(!needs_core(
        AgentKind::Proteus,
        &[ChatKey::Codex("opaque".into())]
    ));
    assert!(needs_core(
        AgentKind::Codex,
        &[ChatKey::Proteus("/sessions/live".into())]
    ));
}

#[test]
fn restart_keeps_core_owner_context_when_child_is_command_target_behind_file() {
    let server = Server::start_in(&[
        ("/sessions/root", "/root-project"),
        ("/sessions/child", "/child-project"),
        ("/sessions/inactive", "/inactive-project"),
    ]);
    let mut spaces = ChatSpaces::default();
    spaces.select("chat:/sessions/inactive");
    spaces.select("chat:/sessions/root");
    let mut layout = spaces.active_layout().clone();
    layout.open("chat:/sessions/child", |_| false);
    layout.open("files:src/main.rs", |_| false);
    spaces.save_active(layout.clone());
    let required = required_chats_with_owner(spaces.active_layout(), spaces.active_root());
    assert_eq!(
        required,
        [
            ChatKey::Proteus("/sessions/child".into()),
            ChatKey::Proteus("/sessions/root".into()),
        ]
    );
    let boot = restore_core(server.api(), None, layout.clone(), true, &required).unwrap();
    assert_eq!(boot.resumed, paths(&["/sessions/child", "/sessions/root"]));
    assert_eq!(
        boot.projects.get(Path::new("/sessions/root")),
        Some(&PathBuf::from("/root-project"))
    );
    assert!(boot.startup.is_none());
    assert_eq!(spaces.active_layout(), &layout);
    assert_eq!(
        server.calls(),
        [
            ("/resume".into(), json!("/sessions/child")),
            ("/resume".into(), json!("/sessions/root")),
        ]
    );
}

#[test]
fn context_only_root_resumes_without_reopening_its_chat_tab() {
    let server = Server::start(&["/sessions/root"]);
    let mut fragment = TabLayout::default();
    fragment.open("files:src/main.rs", |_| false);
    let mut spaces = ChatSpaces::default();
    spaces.set_active(Some("chat:/sessions/root"), fragment.clone());
    let required = required_chats_with_owner(spaces.active_layout(), spaces.active_root());
    assert_eq!(required, [ChatKey::Proteus("/sessions/root".into())]);
    let boot = restore_core(server.api(), None, fragment.clone(), true, &required).unwrap();
    assert_eq!(boot.resumed, paths(&["/sessions/root"]));
    assert!(boot.restored.is_empty());
    assert!(boot.startup.is_none());
    assert_eq!(spaces.active_layout(), &fragment);
    assert!(!spaces.contains("chat:/sessions/root"));
}

#[test]
fn codex_restart_resumes_child_and_owner_but_no_inactive_area() {
    let mut spaces = ChatSpaces::default();
    spaces.select("chat:/sessions/inactive-core");
    spaces.select("codex:opaque:inactive");
    spaces.select("codex:opaque:root");
    let mut layout = spaces.active_layout().clone();
    layout.open("codex:opaque:child", |_| false);
    layout.open("files:src/main.rs", |_| false);
    spaces.save_active(layout);
    let boot = boot_spaces_with(
        &codex_options(),
        spaces,
        |_| panic!("Inactive Core area must not launch Core"),
        |_, _| Ok(codex_peer_many(&["opaque:child", "opaque:root"], false)),
    )
    .unwrap();
    assert!(boot.api.is_none());
    assert!(boot.restored.is_empty());
    assert_eq!(
        boot.codex_opened
            .iter()
            .map(|opened| opened.metadata.id.as_str())
            .collect::<Vec<_>>(),
        ["opaque:child", "opaque:root"]
    );
}

#[test]
fn visible_owner_is_subscribed_once_even_when_it_is_also_command_target() {
    let mut spaces = ChatSpaces::default();
    spaces.select("codex:opaque:root");
    assert_eq!(
        required_chats_with_owner(spaces.active_layout(), spaces.active_root()),
        [ChatKey::Codex("opaque:root".into())]
    );
}

fn codex_options() -> LaunchOptions {
    LaunchOptions {
        workspace: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
        target: crate::launch::Target::Spawn { profile: None },
        agent: AgentKind::Codex,
    }
}

/// This peer rejects catalog discovery and any unexpected history expansion.
fn codex_peer(expected_resume: &str, fail: bool) -> CodexService {
    let expected: Vec<_> = (!expected_resume.is_empty())
        .then_some(expected_resume)
        .into_iter()
        .collect();
    codex_peer_many(&expected, fail)
}

fn codex_peer_many(expected_resume: &[&str], fail: bool) -> CodexService {
    const PEER: &str = r#"
import json,sys
expected,fail=json.loads(sys.argv[1]),sys.argv[2]=='true'
for line in sys.stdin:
    msg=json.loads(line)
    if 'id' not in msg: continue
    method=msg.get('method'); p=msg.get('params',{})
    if method=='initialize': result={}
    elif method=='thread/start':
        assert not expected and p['historyMode']=='paginated'
        result={'thread':{'id':'opaque:own-new','cwd':p['cwd'],'status':{'type':'idle'}}}
    elif method=='thread/resume':
        assert p['threadId'] in expected and p['excludeTurns']
        if p['threadId']=='opaque:disconnected': sys.exit(0)
        if fail and 'missing' in p['threadId']:
            print(json.dumps({'id':msg['id'],'error':{'code':404,'message':'synthetic missing thread'}}),flush=True)
            continue
        result={'thread':{'id':p['threadId'],'cwd':'/synthetic','status':{'type':'idle'}}}
    elif method=='thread/items/list':
        assert p['threadId'] in expected and p['limit']==50 and p['sortDirection']=='desc'
        result={'data':[],'nextCursor':None}
    elif method=='model/list': result={'data':[],'nextCursor':None}
    elif method=='thread/unsubscribe': result={}
    else: raise AssertionError('Unexpected startup method: '+method)
    print(json.dumps({'id':msg['id'],'result':result}),flush=True)
"#;
    let mut launch =
        proteus_codex_client::Launch::codex("python3".into(), std::env::current_dir().unwrap());
    launch.args = vec![
        "-u".into(),
        "-c".into(),
        PEER.into(),
        serde_json::to_string(expected_resume).unwrap(),
        fail.to_string(),
    ];
    CodexService::launch(launch).unwrap()
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
        Self::start_in(
            &dirs
                .iter()
                .map(|dir| (*dir, "/workspace"))
                .collect::<Vec<_>>(),
        )
    }

    fn start_in(dirs: &[(&str, &str)]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let catalog: Vec<_> = dirs
            .iter()
            .map(|(dir, workspace)| {
                AppSessionSummary::new(
                    PathBuf::from(dir),
                    proteus_contracts::domain::new_session_id(),
                    PathBuf::from(workspace),
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
        "/bootstrap" => {
            json!({"session_dir":catalog.first().map(|session| &session.session_dir),"cwd":"/workspace"})
        }
        "/sessions" => serde_json::to_value(catalog).unwrap(),
        "/resume" => json!({
            "type": "response", "id": body["id"], "ok": true, "error": null,
            "output": {"session_dir": dir, "workspace_path": catalog.iter().find(|summary| summary.session_dir == Path::new(dir.as_str().unwrap())).map(|summary| &summary.workspace_path).unwrap(), "activity": {
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
