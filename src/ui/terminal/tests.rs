use super::*;
use base64::Engine as _;
use proteus_contracts::contracts::ToolTerminalSnapshot;

fn target() -> Target {
    Target {
        owner: crate::tabs::chat(std::path::Path::new("/owner")),
        session_dir: "/exact/session".into(),
        terminal: ToolTerminalRef {
            id: uuid::Uuid::from_u128(19),
            title: "synthetic command".into(),
            tty: true,
        },
    }
}
fn snapshot(bytes: &[u8]) -> ToolTerminalSnapshot {
    ToolTerminalSnapshot {
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
    }
}
#[::core::prelude::v1::test]
fn document_identity_preserves_exact_session_uuid_and_owner() {
    let first = target();
    assert_eq!(Target::parse(&first.id()), Some(first.clone()));
    let mut other = first.clone();
    other.session_dir = "/another/session".into();
    assert_ne!(first.id(), other.id());
    other = first.clone();
    other.terminal.id = uuid::Uuid::from_u128(20);
    assert_ne!(first.id(), other.id());
    other = first;
    other.session_dir = "relative".into();
    assert!(Target::parse(&other.id()).is_none());
    assert!(Target::parse("agent-terminal:19").is_none());
}
#[::core::prelude::v1::test]
fn read_validation_is_binary_safe_bounded_and_explicit_about_gaps() {
    let bytes = b"\xff\x00\x1b[31m\xd0";
    let pending = pending::Pending::new(snapshot(bytes), Some(4)).unwrap();
    assert_eq!(pending.bytes.as_deref(), Some(bytes.as_slice()));
    assert!(
        pending.snapshot.data_base64.is_empty(),
        "no retained duplicate encoding"
    );
    assert!(!pending.reset);
    assert!(pending::Pending::new(snapshot(bytes), Some(3)).is_err());
    let mut gap = snapshot(bytes);
    gap.gap = true;
    assert!(pending::Pending::new(gap, Some(0)).unwrap().reset);
    let mut wrong_count = snapshot(bytes);
    wrong_count.next_cursor -= 1;
    assert!(pending::Pending::new(wrong_count, Some(4)).is_err());
    assert!(pending::Pending::new(snapshot(&vec![0; 65_537]), Some(4)).is_err());
}

#[::core::prelude::v1::test]
fn pipe_newlines_preserve_split_crlf_and_raw_cursor() {
    let mut first = pending::Pending::new(snapshot(b"first\nsecond\r"), Some(4)).unwrap();
    first.pipe_newlines(false);
    assert_eq!(
        first.bytes.as_deref(),
        Some(b"first\r\nsecond\r".as_slice())
    );
    assert_eq!(
        first.snapshot.next_cursor,
        4 + b"first\nsecond\r".len() as u64
    );
    let mut next = pending::Pending::new(snapshot(b"\nthird\n"), Some(4)).unwrap();
    next.pipe_newlines(first.pipe_end.unwrap());
    assert_eq!(next.bytes.as_deref(), Some(b"\nthird\r\n".as_slice()));
    next.pipe_newlines(false);
    assert_eq!(
        next.bytes.as_deref(),
        Some(b"\nthird\r\n".as_slice()),
        "backpressure retry does not normalize twice"
    );
}

#[::core::prelude::v1::test]
fn hidden_closed_replaced_and_foreign_connection_views_cannot_poll() {
    let mut cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = cx.add_empty_window();
    let api = Api::new("http://127.0.0.1:1", "synthetic-never-used");
    let target = target();
    let id = target.id();
    let workspace = window.update(|window, cx| cx.new(|cx| Workspace::test_empty(window, cx)));
    let surface = window.update(|window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.test_set_api(api.clone());
            workspace.spaces.select(&target.owner);
            workspace.layout = workspace.spaces.active_layout().clone();
            workspace.layout.open(&id, |_| false);
            workspace.ensure_agent_terminal(&id, window, cx).unwrap()
        })
    });
    let view = surface.view.downcast::<Remote>().unwrap();
    let entity = view.entity_id();
    assert!(view.read_with(&cx, |view, cx| view.current(entity, cx)
        && view.visible(entity, cx)));
    view.update(&mut cx, |view, cx| {
        view.schedule(cx);
        view.schedule(cx);
        assert!(view.scheduled);
    });
    workspace.update(&mut cx, |workspace, _| {
        workspace.layout.open("fixture:hidden", |_| false)
    });
    cx.run_until_parked();
    cx.executor()
        .advance_clock(crate::ui::motion::TERMINAL_REFRESH);
    cx.run_until_parked();
    view.read_with(&cx, |view, cx| {
        assert!(view.current(entity, cx));
        assert!(!view.visible(entity, cx));
        assert!(
            !view.reading && !view.scheduled,
            "hidden timer cannot start HTTP"
        );
    });
    workspace.update(&mut cx, |workspace, _| {
        workspace.test_set_api(Api::new("http://127.0.0.1:1", "other-generation"))
    });
    assert!(!view.read_with(&cx, |view, cx| view.current(entity, cx)));
    workspace.update(&mut cx, |workspace, _| {
        workspace.test_set_api(api);
        workspace.documents.remove(&id);
    });
    assert!(
        !view.read_with(&cx, |view, cx| view.current(entity, cx)),
        "cached old entity cannot reappear after close"
    );
}

#[::core::prelude::v1::test]
fn finished_process_tail_drains_after_racing_failed_resize() {
    let mut cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let view = cx.new(|cx| {
        Remote::new(
            target(),
            Api::new("http://127.0.0.1:1", "synthetic"),
            WeakEntity::new_invalid(),
            cx,
        )
    });
    view.update(&mut cx, |view, _| {
        let mut first = snapshot(b"first");
        first.finished = true;
        first.end_cursor += 4;
        view.cursor = Some(4);
        view.pending = Some(pending::Pending::new(first, view.cursor).unwrap());
        assert!(view.apply_pending());
        assert!(view.process_finished && !view.finished);
        assert!(!view.writable());
        view.resize_finished(Grid::new(100, 30), true);
        assert!(view.error.is_none(), "late resize cannot stop output reads");
        let mut tail = snapshot(b"tail");
        tail.start_cursor = 9;
        tail.next_cursor = 13;
        tail.end_cursor = 13;
        tail.finished = true;
        view.pending = Some(pending::Pending::new(tail, view.cursor).unwrap());
        assert!(view.apply_pending());
        assert_eq!(view.cursor, Some(13));
        assert!(view.finished);
    });
}
