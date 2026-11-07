use std::io::{BufRead, BufReader};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use proteus_contracts::app_protocol::addons::AppAddonsSnapshot;
use proteus_contracts::app_protocol::config::ConfigSummary;
use proteus_contracts::app_protocol::config_builder::{ConfigBuilderSnapshot, ConfigHistory};
use proteus_contracts::app_protocol::http::{
    ApprovalRequest, CancelRequest, DeleteQueuedMessageRequest, DeleteSessionRequest,
    NewSessionRequest, ResumeSessionRequest, SendRequest, SessionOpened, SetConfigBuilderRequest,
    SetSessionSettingRequest, UserInputRequest,
};
use proteus_contracts::app_protocol::settings::SessionSettingUpdate;
use proteus_contracts::app_protocol::{
    AppBootstrap, AppContextMapSnapshot, AppServerEvent, AppSessionSummary, StdioOutput,
};
use proteus_contracts::contracts::{ApprovalCacheScope, UserInputResponse};
use proteus_contracts::domain::MessageId;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod proxy;
mod terminals;
mod transport;

/// Blocking HTTP client for the app server; call it off the UI thread.
#[derive(Clone)]
pub struct Api {
    connection_id: u64,
    agent: ureq::Agent,
    origin: Arc<str>,
    token: Arc<str>,
}

impl Api {
    pub fn connection_id(&self) -> u64 {
        self.connection_id
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn new(origin: &str, token: &str) -> Self {
        let agent = proxy::server_agent(origin, ureq::Proxy::try_from_env());
        static NEXT_CONNECTION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            connection_id: NEXT_CONNECTION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            agent,
            origin: origin.into(),
            token: token.into(),
        }
    }

    pub fn bootstrap(&self) -> Result<AppBootstrap> {
        self.get("/bootstrap", None)
    }

    pub fn sessions(&self) -> Result<Vec<AppSessionSummary>> {
        self.get("/sessions", None)
    }

    pub fn history_page(
        &self,
        session_dir: &Path,
        cursor: &proteus_contracts::app_protocol::AppTranscriptCursor,
    ) -> Result<proteus_contracts::app_protocol::AppTranscriptPage> {
        self.get_with(
            "/history/page",
            Some(session_dir),
            &[
                ("before", &cursor.before.to_string()),
                ("revision", &cursor.revision),
            ],
        )
    }

    /// Read the saved tail without resuming the session or acquiring its writer.
    pub fn initial_history_page(
        &self,
        session_dir: &Path,
    ) -> Result<proteus_contracts::app_protocol::AppTranscriptPage> {
        self.get("/history/page", Some(session_dir))
    }

    pub fn resume(&self, session_dir: &Path) -> Result<SessionOpened> {
        let body = ResumeSessionRequest {
            id: Some(request_id("resume")),
            session_dir: session_dir.to_path_buf(),
        };
        self.command("/resume", None, &body)
    }

    /// Opens a session with the settings of `source` in `workspace`; each
    /// falls back to the source's, then to the server's own.
    pub fn new_session(
        &self,
        source: Option<&Path>,
        workspace: Option<&Path>,
    ) -> Result<SessionOpened> {
        let body = NewSessionRequest {
            id: Some(request_id("new-session")),
            source_session_dir: source.map(Path::to_path_buf),
            workspace: workspace.map(Path::to_path_buf),
        };
        self.command("/new-session", None, &body)
    }

    /// Deletes a saved session from disk; `false` when it was already gone.
    pub fn delete_session(&self, session_dir: &Path) -> Result<bool> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Deleted {
            deleted: bool,
        }
        let body = DeleteSessionRequest {
            id: Some(request_id("delete-session")),
            session_dir: session_dir.to_path_buf(),
        };
        let deleted: Deleted = self.command("/delete-session", None, &body)?;
        Ok(deleted.deleted)
    }

    pub fn send(&self, request: &SendRequest) -> Result<Value> {
        self.command("/send-async", None, request)
    }

    pub fn cancel(&self, session_dir: &Path, run_id: &str) -> Result<Value> {
        let body = CancelRequest {
            id: Some(request_id("cancel")),
            target_id: run_id.to_owned(),
        };
        self.command("/cancel", Some(session_dir), &body)
    }

    pub fn approve(
        &self,
        session_dir: &Path,
        approval_id: &str,
        approved: bool,
        cache: ApprovalCacheScope,
    ) -> Result<Value> {
        let body = ApprovalRequest {
            id: Some(request_id("approval")),
            approval_id: approval_id.to_owned(),
            approved,
            note: None,
            cache,
        };
        self.command("/approval", Some(session_dir), &body)
    }

    pub fn answer_user_input(
        &self,
        session_dir: &Path,
        request_id_value: &str,
        response: UserInputResponse,
    ) -> Result<Value> {
        let body = UserInputRequest {
            id: Some(request_id("user-input")),
            request_id: request_id_value.to_owned(),
            response,
        };
        self.command("/user-input", Some(session_dir), &body)
    }

    pub fn delete_queued(&self, session_dir: &Path, message_id: MessageId) -> Result<Value> {
        let body = DeleteQueuedMessageRequest {
            id: Some(request_id("queue-delete")),
            session_dir: session_dir.to_path_buf(),
            message_id,
        };
        self.command("/queue/delete", None, &body)
    }

    pub fn config(&self, session_dir: &Path) -> Result<ConfigSummary> {
        self.get("/config", Some(session_dir))
    }

    pub fn local_config(&self, session_dir: &Path) -> Result<ConfigSummary> {
        self.get("/config/local", Some(session_dir))
    }

    pub fn context(&self, session_dir: &Path) -> Result<AppContextMapSnapshot> {
        self.get("/context", Some(session_dir))
    }

    pub fn set_session_setting(
        &self,
        session_dir: &Path,
        setting: SessionSettingUpdate,
    ) -> Result<ConfigSummary> {
        let body = SetSessionSettingRequest {
            id: Some(request_id("setting")),
            session_dir: session_dir.to_path_buf(),
            setting,
        };
        let output: Value = self.command("/session/setting", None, &body)?;
        Ok(serde_json::from_value(
            output
                .get("config")
                .ok_or_else(|| anyhow!("session setting response is missing config"))?
                .clone(),
        )?)
    }

    pub fn config_builder(&self, session_dir: &Path) -> Result<ConfigBuilderSnapshot> {
        self.get("/config/builder", Some(session_dir))
    }

    /// Skill catalogs, MCP discovery and package diagnostics of the assembly.
    pub fn addons(&self, session_dir: &Path) -> Result<AppAddonsSnapshot> {
        self.get("/addons", Some(session_dir))
    }

    /// Bytes of an image the session stored, by its `ImageRef::id`.
    pub fn image(&self, session_dir: &Path, id: &str) -> Result<Vec<u8>> {
        self.image_with_timeout(session_dir, id, Duration::from_secs(30))
    }

    fn image_with_timeout(
        &self,
        session_dir: &Path,
        id: &str,
        timeout: Duration,
    ) -> Result<Vec<u8>> {
        // The shared thumbnail queue must also recover from a stalled body.
        // Scope this deadline to images: the SSE stream remains long-lived.
        let response = self
            .agent
            .get(format!("{}/image", self.origin))
            .header("Authorization", format!("Bearer {}", self.token))
            .query("session_dir", session_dir.to_string_lossy())
            .query("path", id)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .call()?;
        let status = response.status();
        if !status.is_success() {
            let body = response
                .into_body()
                .into_with_config()
                .read_to_string()
                .unwrap_or_default();
            bail!("GET /image failed with {status}: {body}");
        }
        Ok(response
            .into_body()
            .into_with_config()
            .limit(proteus_contracts::domain::MAX_IMAGE_BYTES as u64)
            .read_to_vec()?)
    }

    pub fn config_history(&self, session_dir: &Path) -> Result<ConfigHistory> {
        self.get("/config/history", Some(session_dir))
    }

    /// Persists profile changes; the server answers with the fresh snapshot
    /// or with a plain-text validation error.
    pub fn save_config_builder(
        &self,
        session_dir: &Path,
        request: &SetConfigBuilderRequest,
    ) -> Result<ConfigBuilderSnapshot> {
        let response = self
            .agent
            .post(format!("{}/config/builder", self.origin))
            .header("Authorization", format!("Bearer {}", self.token))
            .query("session_dir", session_dir.to_string_lossy())
            .send_json(request)?;
        let status = response.status();
        let text = response.into_body().into_with_config().read_to_string()?;
        if !status.is_success() {
            bail!("{}", text.trim());
        }
        Ok(serde_json::from_str(&text)?)
    }

    /// Opens the session event stream; blocks until the server answers.
    pub fn events(&self, session_dir: &Path) -> Result<EventStream> {
        let _trace = crate::startup_trace::span("GET /events");
        let response = self
            .agent
            .get(format!("{}/events", self.origin))
            .config()
            .timeout_global(None)
            .timeout_recv_response(Some(Duration::from_secs(10)))
            .timeout_recv_body(None)
            .build()
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "text/event-stream")
            .query("session_dir", session_dir.to_string_lossy())
            .call()?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response
                .into_body()
                .into_with_config()
                .read_to_string()
                .unwrap_or_default();
            bail!("event stream failed with {status}: {body}");
        }
        Ok(EventStream {
            reader: BufReader::new(Box::new(response.into_body().into_reader())),
        })
    }

    fn get<T: DeserializeOwned>(&self, path: &str, session_dir: Option<&Path>) -> Result<T> {
        self.get_with(path, session_dir, &[])
    }

    fn get_with<T: DeserializeOwned>(
        &self,
        path: &str,
        session_dir: Option<&Path>,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let _trace = crate::startup_trace::span(path);
        let mut request = self
            .agent
            .get(format!("{}{path}", self.origin))
            .header("Authorization", format!("Bearer {}", self.token));
        if let Some(dir) = session_dir {
            request = request.query("session_dir", dir.to_string_lossy());
        }
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let response = request.call()?;
        let status = response.status();
        let body = response.into_body().into_with_config().read_to_string()?;
        if !status.is_success() {
            bail!("GET {path} failed with {status}: {body}");
        }
        Ok(serde_json::from_str(&body)?)
    }

    fn command<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        session_dir: Option<&Path>,
        body: &B,
    ) -> Result<T> {
        let _trace = crate::startup_trace::span(path);
        let mut request = self
            .agent
            .post(format!("{}{path}", self.origin))
            .header("Authorization", format!("Bearer {}", self.token));
        if let Some(dir) = session_dir {
            request = request.query("session_dir", dir.to_string_lossy());
        }
        let response = request.send_json(body)?;
        let status = response.status();
        let text = response.into_body().into_with_config().read_to_string()?;
        match serde_json::from_str::<StdioOutput>(&text) {
            Ok(output) => proteus_client_common::response::command_output(output)
                .map_err(|error| anyhow!("{path}: {error}")),
            Err(_) => Err(anyhow!("POST {path} failed with {status}: {text}")),
        }
    }
}

fn request_id(kind: &str) -> String {
    format!("{kind}-{}", uuid::Uuid::new_v4().simple())
}

/// Server-sent `output` events of one `/events` connection.
pub struct EventStream {
    reader: BufReader<Box<dyn std::io::Read + Send>>,
}

pub enum StreamItem {
    Event(Box<AppServerEvent>),
    /// Comment line such as the keep-alive heartbeat.
    Heartbeat,
}

impl EventStream {
    /// Returns the next item, or `None` when the server closed the stream.
    pub fn next(&mut self) -> Option<Result<StreamItem>> {
        let mut data = String::new();
        let mut line = String::new();
        loop {
            line.clear();
            match self.reader.read_line(&mut line) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(error) => return Some(Err(error.into())),
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                if data.is_empty() {
                    continue;
                }
                return Some(parse_output(&data).map(StreamItem::Event));
            }
            if line.starts_with(':') {
                if data.is_empty() {
                    return Some(Ok(StreamItem::Heartbeat));
                }
                continue;
            }
            if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            }
        }
    }
}

fn parse_output(data: &str) -> Result<Box<AppServerEvent>> {
    match serde_json::from_str::<StdioOutput>(data)? {
        StdioOutput::Event { event } => Ok(event),
        StdioOutput::Response {
            error: Some(error), ..
        } => Err(anyhow!("event stream failed: {error}")),
        StdioOutput::Response { .. } => Err(anyhow!("unexpected response on the event stream")),
    }
}

#[cfg(test)]
mod connection_identity_tests {
    use super::Api;
    #[test]
    fn clones_keep_identity_and_replacement_at_same_origin_has_new_identity() {
        let first = Api::new("http://localhost:1", "synthetic");
        let second = Api::new("http://localhost:1", "synthetic");
        assert_eq!(first.connection_id(), first.clone().connection_id());
        assert_ne!(first.connection_id(), second.connection_id());
    }
}

#[cfg(test)]
mod saved_history_tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;

    #[test]
    fn initial_saved_history_is_a_readonly_get_without_resume_or_cursor() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = BufReader::new(socket.try_clone().unwrap());
            let mut first = String::new();
            request.read_line(&mut first).unwrap();
            loop {
                let mut line = String::new();
                request.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            let body = r#"{"revision":"saved","start":0,"total":0,"messages":[],"base":null}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            first
        });
        let mut api = Api::new(&origin, "synthetic");
        api.agent = ureq::Agent::config_builder().proxy(None).build().into();
        let page = api.initial_history_page(Path::new("/saved/child")).unwrap();
        assert_eq!(page.revision, "saved");
        let request = server.join().unwrap();
        assert!(request.starts_with("GET /history/page?"), "{request}");
        assert!(request.contains("session_dir="));
        assert!(!request.contains("before=") && !request.contains("revision="));
        assert!(!request.contains("resume") && !request.contains("events"));
    }
}

#[cfg(test)]
mod image_tests {
    use super::Api;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn deadline_covers_stalled_image_body_without_changing_agent_defaults() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let (release, wait) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            loop {
                line.clear();
                request.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                .unwrap();
            // Leave the body incomplete until the client finishes or the
            // bounded fallback exposes a missing body deadline.
            let _ = wait.recv_timeout(Duration::from_secs(5));
        });
        let mut api = Api::new(&origin, "synthetic");
        api.agent = super::proxy::server_agent(&origin, None);
        let start = Instant::now();
        let result = api.image_with_timeout(
            Path::new("synthetic-session"),
            "synthetic-image",
            Duration::from_millis(150),
        );
        let elapsed = start.elapsed();
        let _ = release.send(());
        server.join().unwrap();
        assert!(result.is_err(), "incomplete image body must fail");
        assert!(
            elapsed < Duration::from_secs(3),
            "deadline did not release the request: {elapsed:?}"
        );
        assert_eq!(
            api.agent.config().timeouts().global,
            Some(Duration::from_secs(30))
        );
    }
}
