//! The local app-server must remain reachable independently of the model's
//! network route. A remote origin keeps the inherited proxy configuration.
use std::time::Duration;

pub(super) fn server_agent(origin: &str, inherited: Option<ureq::Proxy>) -> ureq::Agent {
    server_agent_with_timeout(origin, inherited, Duration::from_secs(30))
}

fn server_agent_with_timeout(
    origin: &str,
    inherited: Option<ureq::Proxy>,
    timeout: Duration,
) -> ureq::Agent {
    let proxy = if proteus_client_common::normalize_local_origin(origin).is_ok() {
        None
    } else {
        inherited
    };
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(timeout))
        .proxy(proxy)
        .build();
    super::transport::agent(config, Duration::from_secs(45))
}

#[cfg(test)]
mod tests {
    use super::server_agent;
    use crate::api::Api;
    use proteus_contracts::app_protocol::http::SendRequest;
    use std::io::{BufRead as _, BufReader, Read as _, Write as _};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    #[test]
    fn sse_heartbeats_reset_idle_reads_without_a_total_stream_deadline() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", server.local_addr().unwrap());
        let (done, wait) = std::sync::mpsc::channel();
        let responder = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).unwrap();
                if n == 0 || line == "\r\n" {
                    break;
                }
            }
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
            for _ in 0..5 {
                socket.write_all(b": heartbeat\n\n").unwrap();
                std::thread::sleep(Duration::from_millis(80));
            }
            let _ = wait.recv_timeout(Duration::from_secs(2));
        });
        let mut api = Api::new(&origin, "synthetic");
        let config = ureq::Agent::config_builder().proxy(None).build();
        api.agent = crate::api::transport::agent(config, Duration::from_millis(250));
        let mut stream = api.events(std::path::Path::new("/session")).unwrap();
        let start = Instant::now();
        for _ in 0..5 {
            assert!(matches!(
                stream.next(),
                Some(Ok(crate::api::StreamItem::Heartbeat))
            ));
        }
        assert!(start.elapsed() >= Duration::from_millis(300));
        assert!(matches!(stream.next(), Some(Err(_))));
        done.send(()).unwrap();
        responder.join().unwrap();
    }

    #[test]
    fn stalled_headers_and_body_expire_after_connection() {
        for partial_body in [false, true] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", server.local_addr().unwrap());
            let (done, wait) = std::sync::mpsc::channel();
            let responder = std::thread::spawn(move || {
                let (mut socket, _) = server.accept().unwrap();
                let mut reader = BufReader::new(socket.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    let read = reader.read_line(&mut line).unwrap();
                    if read == 0 || line == "\r\n" {
                        break;
                    }
                }
                if partial_body {
                    let _ = socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{");
                }
                let _ = wait.recv_timeout(Duration::from_secs(2));
            });
            let mut api = Api::new(&origin, "synthetic");
            api.agent = super::server_agent_with_timeout(&origin, None, Duration::from_millis(150));
            let started = Instant::now();
            assert!(api.bootstrap().is_err());
            assert!(started.elapsed() < Duration::from_secs(2));
            done.send(()).unwrap();
            responder.join().unwrap();
        }
    }

    #[test]
    fn local_send_reaches_server_without_contacting_the_inherited_proxy() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", server.local_addr().unwrap());
        let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
        proxy.set_nonblocking(true).unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let responder = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut socket = loop {
                match server.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if let Ok((mut socket, _)) = proxy.accept() {
                            socket.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                            panic!("local request contacted the inherited proxy");
                        }
                        assert!(Instant::now() < deadline, "local server was bypassed");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            let mut size = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                assert!(!line.is_empty(), "incomplete request headers");
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("content-length")
                {
                    size = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; size];
            reader.read_exact(&mut body).unwrap();
            assert_eq!(first, "POST /send-async HTTP/1.1\r\n");
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["text"], "synthetic send");
            let response = r#"{"type":"response","id":"synthetic","ok":true,"output":{"accepted":true},"error":null}"#;
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).unwrap();
            assert_eq!(
                proxy.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        });
        let mut api = Api::new(&origin, "synthetic");
        // Inject the same inherited setting without changing process-global
        // environment variables while other HTTP tests run concurrently.
        api.agent = server_agent(&origin, Some(ureq::Proxy::new(&proxy_url).unwrap()));
        let result = api.send(&SendRequest {
            id: Some("synthetic".into()),
            text: "synthetic send".into(),
            images: vec![],
            options: Default::default(),
            session_dir: "/synthetic/session".into(),
        });
        responder.join().unwrap();
        assert_eq!(result.unwrap()["accepted"], true);
    }

    #[test]
    fn loopback_variants_bypass_proxy_while_remote_origins_keep_it() {
        let proxy = ureq::Proxy::new("http://127.0.0.1:1").unwrap();
        for origin in [
            "http://localhost:8787",
            "http://127.0.0.1:8787",
            "http://127.0.0.2:8787",
            "http://[::1]:8787",
        ] {
            assert!(
                server_agent(origin, Some(proxy.clone()))
                    .config()
                    .proxy()
                    .is_none()
            );
        }
        for origin in ["http://server.example:8787", "http://192.168.1.2:8787"] {
            assert!(
                server_agent(origin, Some(proxy.clone()))
                    .config()
                    .proxy()
                    .is_some()
            );
        }
    }
}
