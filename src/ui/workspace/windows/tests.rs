use super::*;

#[::core::prelude::v1::test]
fn actual_backend_exit_only_releases_the_matching_shared_connection() {
    let cx = ::gpui::TestAppContext::single();
    let windows = WindowGroup::default();
    let old = Api::new("http://127.0.0.1:1", "synthetic-old");
    let current = Api::new("http://127.0.0.1:1", "synthetic-current");
    windows.set_core(current.clone(), None);

    cx.update(|cx| windows.server_stopped(old.connection_id(), "late old process exit", cx));
    assert!(windows.has_core_connection(current.connection_id()));
    cx.update(|cx| windows.server_stopped(current.connection_id(), "actual process exited", cx));
    assert!(!windows.has_core_connection(current.connection_id()));
    assert!(windows.core_slot().lock().unwrap().is_none());
}

#[::core::prelude::v1::test]
fn actual_project_http_callback_checks_catalog_epoch_before_sidebar_mutation() {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::TcpListener;
    use std::time::Duration;

    // Navigation and catalog ownership are independent. In both cases a newer
    // destination owns focus; only the non-invalidated read may refresh metadata.
    for invalidate_catalog in [false, true] {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(
            &format!("http://{}", server.local_addr().unwrap()),
            "synthetic-project-catalog",
        );
        let project = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let summary = proteus_contracts::app_protocol::AppSessionSummary::new(
            "/synthetic/deleted-session".into(),
            uuid::Uuid::from_u128(903),
            project.clone(),
            1,
            None,
            Some("old catalog row".into()),
        );
        let body = serde_json::to_string(&vec![summary]).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(socket.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap();
            assert_eq!(first, "GET /sessions HTTP/1.1\r\n");
            loop {
                let mut line = String::new();
                assert!(reader.read_line(&mut line).unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
            }
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let mut cx = ::gpui::TestAppContext::single();
        cx.update(|cx| {
            gpui_kit::init(cx);
            prefs::init_in_memory(cx);
            prefs::update(cx, |prefs| {
                prefs.preferred_agent = crate::conversations::AgentKind::Proteus
            });
        });
        let window = cx.add_empty_window();
        let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
        window.update(|window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.windows.set_core(api.clone(), None);
                workspace.open_project(project, window, cx);
                if invalidate_catalog {
                    workspace.session_catalog_epoch =
                        workspace.session_catalog_epoch.wrapping_add(1);
                }
                assert!(workspace.begin_navigation(cx));
            });
        });
        cx.run_until_parked();
        worker.join().unwrap();
        workspace.read_with(&cx, |workspace, _| {
            assert_eq!(
                workspace.api.as_ref().unwrap().connection_id(),
                api.connection_id()
            );
            assert_eq!(workspace.sessions.len(), usize::from(!invalidate_catalog));
            assert!(workspace.active_chat.is_none());
            assert!(workspace.opening.is_empty());
        });
    }
}
