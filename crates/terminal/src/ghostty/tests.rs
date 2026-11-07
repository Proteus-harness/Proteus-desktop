use super::*;

fn wait_screen(term: &super::super::Terminal, mut done: impl FnMut(&Screen) -> bool) -> Screen {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let screen = term.snapshot(BASE);
        if done(&screen) {
            return screen;
        }
        assert!(
            Instant::now() < deadline,
            "terminal did not settle:\n{}",
            screen.text()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn ghostty_pty_output_resize_scroll_and_reap() {
    let mut spawned = Terminal::spawn_shell(
        &std::env::current_dir().unwrap(),
        Grid::new(40, 5),
        Some("/bin/sh"),
    )
    .unwrap();
    spawned
        .terminal
        .write(b"printf '\\033[2J\\033[H'; printf 'ghostty-pty-ok\\n'\r".to_vec());
    wait_screen(&spawned.terminal, |s| s.text().contains("ghostty-pty-ok"));
    spawned.terminal.resize(Grid::new(55, 7));
    spawned
        .terminal
        .write(b"printf '\\033[2J\\033[H'; stty size\r".to_vec());
    wait_screen(&spawned.terminal, |s| {
        s.columns == 55 && s.rows.len() == 7 && s.text().contains("7 55")
    });
    spawned
        .terminal
        .write(b"i=0; while [ $i -lt 50 ]; do printf 'row-%s\\n' $i; i=$((i+1)); done\r".to_vec());
    wait_screen(&spawned.terminal, |s| s.text().contains("row-49"));
    spawned.terminal.scroll(20);
    wait_screen(&spawned.terminal, |s| {
        s.scrolled > 0 && !s.text().contains("row-49")
    });
    spawned
        .terminal
        .write(b"printf 'returned-to-bottom\\n'\r".to_vec());
    wait_screen(&spawned.terminal, |s| {
        s.scrolled == 0 && s.text().contains("returned-to-bottom")
    });
    let signals = spawned.signals;
    drop(spawned.terminal);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match signals.try_recv() {
            Ok(Signal::Exited) => break,
            Ok(Signal::Failed(error)) => panic!("{error}"),
            _ => {
                assert!(Instant::now() < deadline, "PTY owner did not exit");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn with_vt(f: impl FnOnce(&mut Vt, &mut dyn FnMut(&Vt) -> Screen)) {
    let mut vt = Vt::new(Options {
        cols: 24,
        rows: 4,
        max_scrollback: 256 * 1024,
    })
    .unwrap();
    let mut render = RenderState::new().unwrap();
    let mut rows = ghostty_vt::render::RowIterator::new().unwrap();
    let mut cells = ghostty_vt::render::CellIterator::new().unwrap();
    let mut frame =
        |vt: &Vt| snapshot::snapshot(vt, &mut render, &mut rows, &mut cells, BASE).unwrap();
    f(&mut vt, &mut frame);
}

#[test]
fn ghostty_unicode_styles_and_cursor() {
    with_vt(|vt, frame| {
        vt.vt_write("A界e\u{301}🙂".as_bytes());
        let screen = frame(vt);
        assert_eq!(screen.rows[0][0].text, "A界e\u{301}🙂");
        assert_eq!(screen.rows[0][0].width, 6);
        assert_eq!(screen.cursor, Some((0, 6)));
        vt.vt_write(b"\r\n\x1b[1;3;4;7;38;2;1;2;3;48;2;9;8;7mX\x1b[0m");
        let screen = frame(vt);
        let run = &screen.rows[1][0];
        assert_eq!(run.text, "X");
        assert_eq!(run.style.foreground, [9, 8, 7]);
        assert_eq!(run.style.background, Some([1, 2, 3]));
        assert!(run.style.bold && run.style.italic && run.style.underline);
    });
}

#[test]
fn ghostty_alternate_screen_modes_and_query_replies() {
    with_vt(|vt, frame| {
        vt.vt_write(b"main\x1b[?1049h\x1b[2J\x1b[Halternate\x1b[?1h\x1b[?2004h\x1b[6n");
        assert!(frame(vt).text().contains("alternate"));
        assert!(vt.mode(Mode::DECCKM).unwrap());
        assert!(vt.mode(Mode::BRACKETED_PASTE).unwrap());
        assert!(
            vt.events()
                .any(|e| matches!(e, Event::PtyWrite(bytes) if bytes == b"\x1b[1;10R"))
        );
        vt.vt_write(b"\x1b[?1049l");
        let screen = frame(vt);
        assert!(screen.text().contains("main"));
        assert!(!screen.text().contains("alternate"));
    });
}

#[test]
fn ghostty_scrollback_limit_and_reflow() {
    with_vt(|vt, frame| {
        vt.set_scrollback_max_lines(Some(100)).unwrap();
        vt.set_scrollback_max_bytes(Some(128 * 1024)).unwrap();
        for _ in 0..10_000 {
            vt.vt_write(b"0123456789abcdefghij\r\n");
            vt.events().for_each(drop);
        }
        let retained = vt.scrollback_rows().unwrap();
        // Both upstream limits prune whole pages; a small configured line
        // count can retain the rest of its page. Check the stated limits and
        // that another 90k lines do not grow the retained window.
        assert_eq!(vt.scrollback_max_lines().unwrap(), Some(100));
        assert_eq!(vt.scrollback_max_bytes().unwrap(), Some(128 * 1024));
        assert!(retained < 10_000);
        for _ in 0..90_000 {
            vt.vt_write(b"0123456789abcdefghij\r\n");
            vt.events().for_each(drop);
        }
        assert!(vt.scrollback_rows().unwrap() <= retained * 2);
        vt.vt_write(b"\x1b[2J\x1b[Habcdefghijklmnopqrstuv");
        vt.resize(12, 4, 1, 1).unwrap();
        let screen = frame(vt);
        assert_eq!(screen.columns, 12);
        assert!(screen.text().contains("abcdefghijkl\nmnopqrstuv"));
    });
}

#[test]
fn input_admission_counts_bytes_before_queueing() {
    let (wake, _receiver) = UnixStream::pair().unwrap();
    wake.set_nonblocking(true).unwrap();
    let shared = Arc::new(Shared {
        view: Mutex::new(View {
            screen: Screen {
                columns: 2,
                rows: vec![vec![]],
                cursor: None,
                scrolled: 0,
            },
            base: BASE,
            grid: Grid::new(2, 1),
            scroll: 0,
            bottom: false,
        }),
        modes: AtomicU8::new(0),
        input_bytes: AtomicUsize::new(0),
        frame_credit: AtomicBool::new(false),
        shutdown: AtomicBool::new(false),
        exited: AtomicBool::new(false),
        done: AtomicBool::new(false),
        wake,
    });
    let (input, incoming) = mpsc::sync_channel(1);
    let (signals, errors) = async_channel::bounded(2);
    let terminal = Terminal {
        shared: shared.clone(),
        input,
        signals,
    };
    terminal.write(vec![b'x'; INPUT_LIMIT]);
    terminal.write(vec![b'y']);
    assert_eq!(shared.input_bytes.load(Ordering::Acquire), INPUT_LIMIT);
    assert!(
        matches!(incoming.try_recv().unwrap(), Input::Bytes(bytes) if bytes.len() == INPUT_LIMIT)
    );
    assert!(incoming.try_recv().is_err());
    assert!(matches!(errors.try_recv(), Ok(Signal::InputRejected(_))));
    terminal.paste(&"z".repeat(INPUT_LIMIT));
    assert!(matches!(errors.try_recv(), Ok(Signal::InputRejected(_))));
    assert_eq!(shared.input_bytes.load(Ordering::Acquire), INPUT_LIMIT);
}

#[test]
fn ghostty_encoder_follows_cursor_and_kitty_modes() {
    with_vt(|vt, _| {
        let mut encoder = ghostty_vt::key::Encoder::new().unwrap();
        let mut event = ghostty_vt::key::Event::new().unwrap();
        let up = || {
            input::KeyInput::new(crate::Key {
                key: "up",
                ..Default::default()
            })
            .unwrap()
        };
        assert_eq!(
            up().encode(vt, &mut encoder, &mut event).unwrap(),
            b"\x1b[A"
        );
        vt.vt_write(b"\x1b[?1h");
        assert_eq!(
            up().encode(vt, &mut encoder, &mut event).unwrap(),
            b"\x1bOA"
        );
        let ctrl_c = || {
            input::KeyInput::new(crate::Key {
                key: "c",
                ctrl: true,
                ..Default::default()
            })
            .unwrap()
        };
        assert_eq!(
            ctrl_c().encode(vt, &mut encoder, &mut event).unwrap(),
            b"\x03"
        );
        vt.vt_write(b"\x1b[>1u");
        assert_eq!(
            ctrl_c().encode(vt, &mut encoder, &mut event).unwrap(),
            b"\x1b[99;5u"
        );
    });
}

#[test]
fn rejecting_large_paste_keeps_the_shell_alive() {
    let spawned = Terminal::spawn_shell(
        &std::env::current_dir().unwrap(),
        Grid::new(40, 5),
        Some("/bin/sh"),
    )
    .unwrap();
    spawned.terminal.paste(&"x".repeat(INPUT_LIMIT));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match spawned.signals.try_recv() {
            Ok(Signal::InputRejected(_)) => break,
            Ok(Signal::Failed(error)) => panic!("{error}"),
            _ => {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    spawned
        .terminal
        .write(b"printf 'shell-still-alive\\n'\r".to_vec());
    wait_screen(&spawned.terminal, |s| {
        s.text().contains("shell-still-alive")
    });
}

#[test]
#[cfg(target_os = "linux")]
fn dropping_terminal_reaps_background_jobs_with_another_process_group() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target/terminal-fixtures")
        .join(format!(
            "background-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let spawned = Terminal::spawn_shell(&root, Grid::new(60, 5), Some("/bin/sh")).unwrap();
    spawned.terminal.write(
        b"(trap '' HUP; exec sleep 60) & echo $! > child.pid; printf 'job-started\\n'\r".to_vec(),
    );
    wait_screen(&spawned.terminal, |s| {
        s.text().contains("job-started") && root.join("child.pid").exists()
    });
    let pid: u32 = std::fs::read_to_string(root.join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let stat_path = format!("/proc/{pid}/stat");
    let before = std::fs::read_to_string(&stat_path).unwrap();
    let started = before
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned();
    drop(spawned.terminal);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let live = std::fs::read_to_string(&stat_path)
            .ok()
            .is_some_and(|stat| {
                let fields: Vec<_> = stat
                    .rsplit_once(')')
                    .unwrap()
                    .1
                    .split_whitespace()
                    .collect();
                fields[0] != "Z" && fields[19] == started
            });
        if !live {
            break;
        }
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            panic!("background job {pid} survived terminal drop");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn hidden_terminal_keeps_parsing_and_publishes_on_the_next_frame_credit() {
    let spawned = Terminal::spawn_shell(
        &std::env::current_dir().unwrap(),
        Grid::new(60, 5),
        Some("/bin/sh"),
    )
    .unwrap();
    spawned.terminal.write(
        b"awk 'BEGIN { for(i=0;i<12000;i++) print i; print \"hidden-terminal-finished\" }'\r"
            .to_vec(),
    );
    // No snapshot requests while the hidden terminal receives its burst.
    std::thread::sleep(Duration::from_millis(400));
    wait_screen(&spawned.terminal, |s| {
        s.rows.iter().any(|row| {
            row.iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .trim()
                == "hidden-terminal-finished"
        })
    });
}

#[test]
#[cfg(target_os = "linux")]
fn exited_shell_cleans_reparented_jobs_in_its_owned_pty_session() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target/terminal-fixtures")
        .join(format!(
            "orphan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let spawned = Terminal::spawn_shell(&root, Grid::new(60, 5), Some("/bin/sh")).unwrap();
    spawned.terminal.write(b"(trap '' HUP; echo ready > job.ready; exec sleep 60) </dev/null >/dev/null 2>&1 & echo $! > child.pid; while [ ! -f job.ready ]; do :; done; exit\r".to_vec());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !root.join("child.pid").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let pid: u32 = std::fs::read_to_string(root.join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    loop {
        let running = std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .is_some_and(|stat| {
                stat.rsplit_once(')').unwrap().1.split_whitespace().next() != Some("Z")
            });
        if !running {
            break;
        }
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
            panic!("orphaned PTY-session job survived shell exit");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(spawned.terminal);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(target_os = "linux")]
fn eof_publishes_held_final_output_and_retains_readonly_scrollback_after_reaping() {
    let root = std::env::current_dir()
        .unwrap()
        .join("target/terminal-fixtures")
        .join(format!(
            "exit-frame-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
    std::fs::create_dir_all(&root).unwrap();
    let mut spawned = Terminal::spawn_shell(&root, Grid::new(60, 5), Some("/bin/sh")).unwrap();
    let super::super::Terminal::Ghostty(terminal) = &spawned.terminal else {
        unreachable!()
    };
    let shared = terminal.shared.clone();
    spawned
        .terminal
        .write(b"stty -echo; echo $$ > shell.pid; printf '\\033[2J\\033[Hready\\n'\r".to_vec());
    wait_screen(&spawned.terminal, |screen| {
        root.join("shell.pid").exists() && screen.text().contains("ready")
    });
    let pid: u32 = std::fs::read_to_string(root.join("shell.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let started = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .unwrap()
        .rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .to_owned();
    shared.frame_credit.store(false, Ordering::Release);
    spawned.terminal.write(b"printf '\\033[?2026h'; i=0; while [ $i -lt 120 ]; do printf 'exit-row-%s\\n' $i; i=$((i+1)); done; printf 'FINAL_EOF_OUTPUT\\n'; exit 0\r".to_vec());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match spawned.signals.try_recv() {
            Ok(Signal::Exited) => break,
            Ok(Signal::Failed(error)) => panic!("{error}"),
            _ => {
                assert!(Instant::now() < deadline, "shell did not exit");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    // Do not request a new frame credit: the terminal boundary must have
    // published this final output despite the held rendering and hidden view.
    let screen = shared.view.lock().unwrap().screen.clone();
    assert!(
        screen.text().contains("FINAL_EOF_OUTPUT"),
        "{}",
        screen.text()
    );
    assert_eq!(screen.cursor, None);
    assert!(shared.exited.load(Ordering::Acquire));
    assert!(!shared.done.load(Ordering::Acquire));
    assert!(
        !std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .ok()
            .is_some_and(|stat| {
                stat.rsplit_once(')').unwrap().1.split_whitespace().nth(19)
                    == Some(started.as_str())
            }),
        "the owned shell must be reaped before Exited is published"
    );
    assert!(!matches!(spawned.signals.try_recv(), Ok(Signal::Exited)));
    spawned
        .terminal
        .write(b"must-not-reach-a-new-process\r".to_vec());
    assert!(!spawned.terminal.key(super::super::Key {
        key: "a",
        text: Some("a"),
        ctrl: false,
        alt: false,
        shift: false,
    }));
    assert_eq!(shared.input_bytes.load(Ordering::Acquire), 0);
    spawned.terminal.scroll(80);
    let scrolled = wait_screen(&spawned.terminal, |screen| {
        screen.scrolled > 0 && !screen.text().contains("FINAL_EOF_OUTPUT")
    });
    assert!(scrolled.text().contains("exit-row-"));
    assert_eq!(scrolled.cursor, None);
    spawned.terminal.resize(Grid::new(45, 7));
    let resized = wait_screen(&spawned.terminal, |screen| {
        screen.columns == 45 && screen.rows.len() == 7
    });
    assert_eq!(resized.cursor, None);
    let changed_base = Base {
        foreground: [1, 230, 23],
        background: [20, 30, 40],
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let screen = spawned.terminal.snapshot(changed_base);
        if screen
            .rows
            .iter()
            .flatten()
            .any(|run| run.style.foreground == changed_base.foreground)
        {
            assert_eq!(screen.cursor, None);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "retained screen ignored the theme change"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(spawned.terminal);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !shared.done.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "read-only actor did not stop with its view"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!matches!(spawned.signals.try_recv(), Ok(Signal::Exited)));
    std::fs::remove_dir_all(root).unwrap();
}
