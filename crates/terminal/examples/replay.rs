//! Headless PTY resource fixture; each engine runs in a fresh process.
use proteus_terminal::{Base, Grid, Signal, Terminal};
use std::time::{Duration, Instant};

fn memory() -> anyhow::Result<(usize, usize, usize)> {
    let status = std::fs::read_to_string("/proc/self/status")?;
    let smaps = std::fs::read_to_string("/proc/self/smaps_rollup")?;
    let kb = |text: &str, key: &str| -> usize {
        text.lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    };
    Ok((
        kb(&status, "VmRSS:"),
        kb(&smaps, "Pss:"),
        kb(&status, "VmHWM:"),
    ))
}

fn main() -> anyhow::Result<()> {
    let lines: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "10000".into())
        .parse()?;
    let start = Instant::now();
    let spawned = Terminal::spawn(&std::env::current_dir()?, Grid::new(100, 30))?;
    let spawned_ms = start.elapsed().as_secs_f64() * 1000.;
    let command = format!(
        "awk 'BEGIN {{ for(i=0;i<{lines};i++) printf \"\\033[32mrow-%06d\\033[0m ASCII кириллица 界 emoji🙂 0123456789abcdefghijklmnopqrstuvwxyz\\n\",i; print \"REPLAY_DONE\" }}'\r"
    );
    spawned.terminal.write(command.into_bytes());
    let base = Base {
        foreground: [200; 3],
        background: [0; 3],
    };
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let screen = spawned.terminal.snapshot(base);
        if screen.rows.iter().any(|row| {
            row.iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .trim()
                == "REPLAY_DONE"
        }) {
            break;
        }
        anyhow::ensure!(Instant::now() < deadline, "replay timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
    let replay_ms = start.elapsed().as_secs_f64() * 1000.;
    let immediate = memory()?;
    // Same idle interval for both engines; Ghostty's incremental compression
    // starts after 250 ms. Keep the same live shell for both memory samples.
    std::thread::sleep(Duration::from_millis(1000));
    let idle = memory()?;
    let output = format!(
        "{{\"engine\":\"{}\",\"lines\":{},\"spawn_ms\":{:.3},\"replay_ms\":{:.3},\"immediate_rss_kib\":{},\"immediate_pss_kib\":{},\"idle_rss_kib\":{},\"idle_pss_kib\":{},\"peak_rss_kib\":{}}}",
        std::env::var("PROTEUS_TERMINAL_ENGINE").unwrap_or_else(|_| "alacritty".into()),
        lines,
        spawned_ms,
        replay_ms,
        immediate.0,
        immediate.1,
        idle.0,
        idle.1,
        idle.2
    );
    drop(spawned.terminal);
    proteus_terminal::shutdown_all();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match spawned.signals.try_recv() {
            Ok(Signal::Exited) | Err(async_channel::TryRecvError::Closed) => break,
            Ok(Signal::Failed(error)) => anyhow::bail!("{error}"),
            _ => {
                anyhow::ensure!(Instant::now() < deadline, "PTY cleanup timed out");
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    println!("{output}");
    Ok(())
}
