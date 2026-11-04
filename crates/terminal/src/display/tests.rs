use super::*;

fn engines() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut engines = vec!["alacritty"];
    #[cfg(all(feature = "ghostty-vt", unix))]
    engines.push("ghostty");
    engines
}

fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "display did not make progress");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn screen(display: &Display, condition: impl Fn(&Screen) -> bool) -> Screen {
    let mut found = None;
    wait(|| {
        let value = display.snapshot(BASE);
        if condition(&value) {
            found = Some(value);
            true
        } else {
            false
        }
    });
    found.unwrap()
}

fn key<'a>(name: &'a str, text: Option<&'a str>) -> Key<'a> {
    Key {
        key: name,
        text,
        ctrl: false,
        alt: false,
        shift: false,
    }
}

fn input(receiver: &async_channel::Receiver<Vec<u8>>) -> Vec<u8> {
    let mut bytes = None;
    wait(|| match receiver.try_recv() {
        Ok(value) => {
            bytes = Some(value);
            true
        }
        Err(_) => false,
    });
    bytes.unwrap()
}

#[test]
fn split_binary_utf8_and_ansi_preserve_parser_state() {
    for engine in engines() {
        let spawned = Display::with_engine(Grid::new(20, 3), engine).unwrap();
        for bytes in [
            b"\x1b[38;2;1;".as_slice(),
            b"2;3m\xd0",
            b"\x9f\xd1",
            b"\x80\x1b[0",
            b"m\r\nready",
        ] {
            assert!(spawned.display.feed(bytes.to_vec()));
        }
        let found = screen(&spawned.display, |screen| screen.text().contains("ready"));
        assert!(found.text().contains("Пр"), "{engine}: {}", found.text());
        assert!(!found.text().contains('\u{fffd}'));
        assert!(
            found.rows[0]
                .iter()
                .any(|run| run.text.contains("Пр") && run.style.foreground == [1, 2, 3])
        );
        assert_eq!(found.cursor, Some((1, 5)));
    }
}

#[test]
fn typed_password_and_paste_only_reach_user_input_queue() {
    for engine in engines() {
        let spawned = Display::with_engine(Grid::new(40, 3), engine).unwrap();
        assert!(spawned.display.feed(b"Password: \x1b[6n\x1b[c".to_vec()));
        screen(&spawned.display, |screen| {
            screen.text().contains("Password:")
        });
        assert!(spawned.input.is_empty(), "VT replies must be ignored");
        let password = "synthetic-not-a-password";
        assert!(spawned.display.key(key("s", Some(password))));
        assert_eq!(input(&spawned.input), password.as_bytes());
        assert!(spawned.display.key(key("enter", None)));
        assert_eq!(input(&spawned.input), b"\r");
        let found = spawned.display.snapshot(BASE);
        assert!(!found.text().contains(password), "no local echo: {engine}");
        assert!(spawned.display.feed(b"\r\naccepted\x1b[?2004h".to_vec()));
        screen(&spawned.display, |screen| {
            screen.text().contains("accepted")
        });
        spawned.display.paste("hello\nworld\x1b[201~");
        assert_eq!(input(&spawned.input), b"\x1b[200~hello\rworld\x1b[201~");
        assert!(!spawned.display.snapshot(BASE).text().contains("hello"));
    }
}

#[test]
fn key_modes_and_unknown_key_admission() {
    for engine in engines() {
        let spawned = Display::with_engine(Grid::new(20, 3), engine).unwrap();
        assert!(!spawned.display.key(key("shift", None)));
        assert!(spawned.display.key(key("up", None)));
        assert_eq!(input(&spawned.input), b"\x1b[A");
        assert!(spawned.display.feed(b"\x1b[?1hMODE".to_vec()));
        screen(&spawned.display, |screen| screen.text().contains("MODE"));
        assert!(spawned.display.key(key("up", None)));
        assert_eq!(input(&spawned.input), b"\x1bOA");
        let mut ctrl_c = key("c", None);
        ctrl_c.ctrl = true;
        assert!(spawned.display.key(ctrl_c));
        assert_eq!(input(&spawned.input), b"\x03");
    }
}

#[test]
fn reset_discards_incomplete_escape_and_scrollback() {
    for engine in engines() {
        let spawned = Display::with_engine(Grid::new(20, 2), engine).unwrap();
        assert!(
            spawned
                .display
                .feed(b"old\r\nolder\r\noldest\x1b[38;2;".to_vec())
        );
        assert!(spawned.display.reset());
        assert!(spawned.display.feed(b"fresh".to_vec()));
        let found = screen(&spawned.display, |screen| screen.text().contains("fresh"));
        assert!(!found.text().contains("old"));
        assert_eq!(found.cursor, Some((0, 5)));
        spawned.display.scroll(100);
        screen(&spawned.display, |screen| {
            screen.text().contains("fresh") && screen.scrolled == 0
        });
    }
}

#[test]
fn resize_scroll_and_title_bell_are_display_only() {
    for engine in engines() {
        let mut spawned = Display::with_engine(Grid::new(20, 2), engine).unwrap();
        assert!(
            spawned
                .display
                .feed(b"a\r\nb\r\nc\r\nd\x1b]2;remote\x07\x07".to_vec())
        );
        screen(&spawned.display, |screen| screen.text().contains('d'));
        spawned.display.scroll(1);
        screen(&spawned.display, |screen| screen.scrolled > 0);
        spawned.display.resize(Grid::new(30, 4));
        screen(&spawned.display, |screen| {
            screen.columns == 30 && screen.rows.len() == 4
        });
        let mut signals = vec![];
        while let Ok(signal) = spawned.signals.try_recv() {
            signals.push(signal);
        }
        assert!(
            signals.contains(&Signal::Title(Some("remote".into()))),
            "{engine}: {signals:?}"
        );
        assert!(signals.contains(&Signal::Bell));
        assert!(spawned.input.is_empty());
    }
}

#[test]
fn queues_reject_whole_input_without_retaining_payload_in_signal() {
    let (sender, receiver) = async_channel::bounded(COMMANDS);
    let (signals, signal_rx) = async_channel::bounded(16);
    for _ in 0..COMMANDS - 1 {
        sender.try_send(vec![b'x'; INPUT_CHUNK]).unwrap();
    }
    deliver_input(vec![b'y'; INPUT_CHUNK + 1], &sender, &signals);
    assert_eq!(
        receiver.len(),
        COMMANDS - 1,
        "paste must not be partially delivered"
    );
    assert!(matches!(
        signal_rx.try_recv().unwrap(),
        Signal::InputRejected(_)
    ));
    deliver_input(vec![b'z'], &sender, &signals);
    assert_eq!(receiver.len(), COMMANDS);

    for engine in engines() {
        let spawned = Display::with_engine(Grid::new(20, 2), engine).unwrap();
        let oversized = vec![0; QUEUE_BYTES + 1];
        let allocation = oversized.as_ptr();
        let rejected = spawned.display.try_feed(oversized).unwrap_err();
        assert_eq!(
            rejected.as_ptr(),
            allocation,
            "backpressure returns the same allocation"
        );
        assert!(!spawned.display.feed(vec![0; QUEUE_BYTES + 1]));
        spawned.display.paste(&"synthetic".repeat(QUEUE_BYTES / 8));
        assert!(
            matches!(spawned.signals.try_recv().unwrap(), Signal::InputRejected(message) if !message.contains("synthetic"))
        );
        assert!(spawned.display.feed(b"healthy".to_vec()));
        screen(&spawned.display, |screen| screen.text().contains("healthy"));
        spawned.display.paste("ok");
        assert_eq!(input(&spawned.input), b"ok");
    }
}

#[test]
fn hidden_bursts_coalesce_and_drop_closes_without_process_exit() {
    for engine in engines() {
        let spawned = Display::with_engine(Grid::new(20, 2), engine).unwrap();
        for _ in 0..20 {
            assert!(spawned.display.feed(b"\r\nline".to_vec()));
        }
        assert!(spawned.display.feed(b"\r\nlast".to_vec()));
        screen(&spawned.display, |screen| screen.text().contains("last"));
        assert!(spawned.redraw.len() <= 1);
        drop(spawned.display);
        wait(|| {
            spawned.input.is_closed() && spawned.redraw.is_closed() && spawned.signals.is_closed()
        });
        while let Ok(signal) = spawned.signals.try_recv() {
            assert!(!matches!(signal, Signal::Exited | Signal::Failed(_)));
        }
    }
}

#[cfg(all(feature = "ghostty-vt", unix))]
#[test]
fn ghostty_uses_maintained_kitty_keyboard_encoder() {
    let spawned = Display::with_engine(Grid::new(20, 2), "ghostty").unwrap();
    assert!(spawned.display.feed(b"\x1b[>1uKITTY".to_vec()));
    screen(&spawned.display, |screen| screen.text().contains("KITTY"));
    let mut ctrl_c = key("c", Some("c"));
    ctrl_c.ctrl = true;
    assert!(spawned.display.key(ctrl_c));
    assert_eq!(input(&spawned.input), b"\x1b[99;5u");
}
