//! A terminal child remains a document even when its saved tail fails to load.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;

fn serve_tail() -> (Api, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let api = Api::new(
        &format!("http://{}", listener.local_addr().unwrap()),
        "fixture",
    );
    let server = std::thread::spawn(move || {
        let row = r#"{"revision":"saved","start":0,"total":1,"messages":[{"role":"user","text":"Recovered","images":[],"streaming":false}]}"#;
        for (status, body) in [
            ("503 Unavailable", "temporarily unavailable"),
            ("200 OK", row),
        ] {
            let (mut socket, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&socket);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            assert_eq!(
                line.replace("%2F", "/"),
                "GET /history/page?session_dir=/saved/child HTTP/1.1\r\n"
            );
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
            }
            write!(socket, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    (api, server)
}

fn open(cx: &mut VisualTestContext, api: Api) -> (Entity<Workspace>, Entity<Chat>, String) {
    let parent = crate::tabs::chat(std::path::Path::new("/saved/parent"));
    let id = Target {
        owner: parent.clone(),
        parent: parent.clone(),
        child_thread_id: Some("finished".into()),
        session: AppSubagentSession {
            session_dir: "/saved/child".into(),
            session_id: uuid::Uuid::from_u128(42),
        },
    }
    .id();
    cx.update(|window, cx| {
        let workspace = cx.new(|cx| Workspace::test_empty(window, cx));
        let chat = workspace.update(cx, |ws, cx| {
            ws.test_set_api(api);
            // A detached document pane owns no parent runtime/catalog requests.
            ws.layout.open(&id, |_| false);
            assert!(ws.spaces.set_active(Some(&parent), ws.layout.clone()));
            ws.ensure_subagent_document(&id, window, cx)
                .unwrap()
                .view
                .downcast::<Chat>()
                .ok()
                .unwrap()
        });
        chat.update(cx, |chat, cx| {
            chat.readonly_source_changed(Some("completed".into()), false, false, cx)
        });
        (workspace, chat, id)
    })
}

fn app() -> TestAppContext {
    let cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    cx
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|w, cx| w.draw(cx).clear(cx));
    cx.run_until_parked();
}

#[::core::prelude::v1::test]
fn failed_finished_child_retries_in_place_only_on_request() {
    let (api, server) = serve_tail();
    let mut app = app();
    let cx = app.add_empty_window();
    let (_workspace, chat, _id) = open(cx, api);
    cx.update(|window, cx| {
        window.replace_root(cx, |window, cx| {
            gpui_kit::component::Root::new(chat.clone(), window, cx)
        })
    });
    draw(cx);
    cx.executor()
        .advance_clock(crate::ui::motion::SAVED_CHAT_REFRESH * 3);
    draw(cx);
    chat.read_with(cx, |chat, _| {
        let binding = chat.readonly.as_ref().unwrap();
        assert!(binding.error.as_ref().unwrap().contains("503"));
        assert!(!binding.loaded && !binding.refresh_pending && binding.inflight.is_none());
    });
    let retry = cx
        .debug_bounds("readonly-history-retry")
        .expect("in-place retry")
        .center();
    cx.simulate_click(retry, Default::default());
    draw(cx);
    draw(cx);
    assert!(cx.debug_bounds("readonly-history-retry").is_none());
    chat.read_with(cx, |chat, _| {
        assert_eq!(
            chat.display_transcript().entries()[0].text.as_ref(),
            "Recovered"
        );
        let binding = chat.readonly.as_ref().unwrap();
        assert!(binding.loaded && binding.error.is_none() && !binding.refresh_pending);
        assert!(chat._pump.is_none());
    });
    server.join().unwrap();
}

#[::core::prelude::v1::test]
fn failed_finished_child_retry_cannot_apply_after_close_or_revision_change() {
    for close in [true, false] {
        let (api, server) = serve_tail();
        let mut app = app();
        let cx = app.add_empty_window();
        let (ws, chat, id) = open(cx, api);
        cx.update(|_, cx| chat.update(cx, |chat, cx| chat.ensure_readonly_history(cx)));
        cx.run_until_parked();
        cx.update(|window, cx| {
            chat.update(cx, |chat, cx| {
                assert!(chat.readonly.as_ref().unwrap().error.is_some());
                if close {
                    chat.retry_failed_readonly_history(cx);
                    chat.ensure_readonly_history(cx);
                } else {
                    // A lifecycle read can start before the old error button is repainted.
                    chat.readonly_source_changed(Some("failed".into()), false, true, cx);
                }
                let admitted = chat.readonly.as_ref().unwrap().inflight.unwrap();
                chat.retry_failed_readonly_history(cx);
                chat.ensure_readonly_history(cx);
                assert_eq!(chat.readonly.as_ref().unwrap().inflight, Some(admitted));
                assert!(!chat.readonly.as_ref().unwrap().dirty, "no trailing retry");
                if !close {
                    chat.show_latest(cx);
                }
            });
            if close {
                ws.update(cx, |ws, cx| ws.close_tab(&id, window, cx));
            }
        });
        cx.run_until_parked();
        let entity = chat.entity_id();
        chat.read_with(cx, |chat, cx| {
            let binding = chat.readonly.as_ref().unwrap();
            assert!(!binding.loaded && binding.inflight.is_none());
            assert!(chat.display_transcript().entries().is_empty());
            assert_eq!(ws.read(cx).document_current(&id, entity), !close);
        });
        server.join().unwrap();
    }
}
