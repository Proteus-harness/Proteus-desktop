//! Owned PTY-session cleanup, including reparented background jobs.
use anyhow::{Context as _, Result};
use portable_pty::{Child, MasterPty};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

pub(super) struct Process {
    pub master: Box<dyn MasterPty + Send>,
    child: Box<dyn Child + Send + Sync>,
    session: Option<OwnedChild>,
}

impl Process {
    pub fn new(
        master: Box<dyn MasterPty + Send>,
        child: Box<dyn Child + Send + Sync>,
    ) -> Result<Self> {
        let mut process = Self {
            master,
            child,
            session: None,
        };
        // portable-pty creates the child as a new Unix session leader. Keep
        // its identity while the process is owned and remains unreaped.
        #[cfg(target_os = "linux")]
        {
            let pid = process.child.process_id().context("PTY shell has no PID")?;
            process.session = Some(
                OwnedChild::capture(pid, pid)
                    .context("не удалось открыть pidfd оболочки терминала")?,
            );
        }
        Ok(process)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let children = self
            .session
            .as_ref()
            .filter(|owner| start_time(owner.pid).as_deref() == Some(&owner.started))
            .map(|owner| session_members(owner.pid))
            .unwrap_or_default();
        for child in &children {
            child.signal(libc::SIGHUP);
        }
        if let Some(owner) = &self.session {
            owner.signal(libc::SIGHUP);
        } else {
            let _ = self.child.kill();
        }
        let deadline = Instant::now() + Duration::from_millis(150);
        while Instant::now() < deadline {
            let exited = self.child.try_wait().ok().flatten().is_some();
            if exited && children.iter().all(|child| !child.alive()) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // pidfds refer to precisely the captured processes even if they exit
        // and their numeric PIDs are reused before this escalation.
        for child in &children {
            child.signal(libc::SIGKILL);
        }
        if let Some(owner) = &self.session {
            owner.signal(libc::SIGKILL);
        } else if self.child.try_wait().ok().flatten().is_none() {
            if let Some(pid) = self.child.process_id() {
                // Only the still-owned, unreaped child is addressed by PID.
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
            }
        }
        let _ = self.child.wait();
    }
}

struct OwnedChild {
    pid: u32,
    started: String,
    pidfd: OwnedFd,
}
impl OwnedChild {
    #[cfg(target_os = "linux")]
    fn capture(pid: u32, expected_session: u32) -> Option<Self> {
        let (before, session) = identity(pid)?;
        if session != expected_session {
            return None;
        }
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) } as i32;
        if fd < 0 {
            return None;
        }
        // SAFETY: pidfd_open returned a new, owned descriptor.
        let pidfd = unsafe { OwnedFd::from_raw_fd(fd) };
        let (started, session) = identity(pid)?;
        (before == started && session == expected_session).then_some(Self {
            pid,
            started,
            pidfd,
        })
    }
    fn signal(&self, signal: i32) {
        #[cfg(target_os = "linux")]
        unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                self.pidfd.as_raw_fd(),
                signal,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            );
        }
    }
    fn alive(&self) -> bool {
        let mut poll = libc::pollfd {
            fd: self.pidfd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut poll, 1, 0) == 0 }
    }
}

#[cfg(target_os = "linux")]
fn start_time(pid: u32) -> Option<String> {
    identity(pid).map(|(started, _)| started)
}
#[cfg(target_os = "linux")]
fn identity(pid: u32) -> Option<(String, u32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<_> = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .take(20)
        .collect();
    (fields.len() == 20)
        .then(|| Some((fields[19].to_owned(), fields[3].parse::<u32>().ok()?)))
        .flatten()
}
#[cfg(not(target_os = "linux"))]
fn start_time(_: u32) -> Option<String> {
    None
}

#[cfg(target_os = "linux")]
fn session_members(session: u32) -> Vec<OwnedChild> {
    let mut children = Vec::new();
    let Ok(processes) = std::fs::read_dir("/proc") else {
        return children;
    };
    for process in processes.flatten() {
        let Some(pid) = process
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == session {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(process.path().join("stat")) else {
            continue;
        };
        let Some((_, fields)) = stat.rsplit_once(')') else {
            continue;
        };
        let Some(sid) = fields
            .split_whitespace()
            .nth(3)
            .and_then(|v| v.parse::<u32>().ok())
        else {
            continue;
        };
        if sid == session {
            if let Some(child) = OwnedChild::capture(pid, session) {
                children.push(child);
            }
            if children.len() >= 4096 {
                break;
            }
        }
    }
    children
}
#[cfg(not(target_os = "linux"))]
fn session_members(_: u32) -> Vec<OwnedChild> {
    Vec::new()
}
