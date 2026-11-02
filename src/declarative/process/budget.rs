//! The same reservation ownership as codex-client's transport, with a count
//! limit and interruptible backpressure for extension stdout.

use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};

use anyhow::{Result, bail};

#[derive(Default)]
struct Used {
    count: usize,
    bytes: usize,
    #[cfg(test)]
    waiters: usize,
}

pub(super) struct Budget {
    used: Mutex<Used>,
    available: Condvar,
    count: usize,
    bytes: usize,
}

impl Budget {
    pub(super) fn new(count: usize, bytes: usize) -> Arc<Self> {
        Arc::new(Self {
            used: Mutex::default(),
            available: Condvar::new(),
            count,
            bytes,
        })
    }

    fn fits(&self, used: &Used, bytes: usize) -> bool {
        used.count < self.count && bytes <= self.bytes.saturating_sub(used.bytes)
    }

    /// No waiting on the UI thread; admission precedes allocating encoded JSON.
    pub(super) fn try_reserve(
        self: &Arc<Self>,
        bytes: usize,
        closed: &AtomicBool,
    ) -> Result<Permit> {
        let mut used = self.used.lock().unwrap();
        if closed.load(Ordering::Acquire) {
            bail!("транспорт расширения закрыт");
        }
        if !self.fits(&used, bytes) {
            bail!("очередь сообщений расширений заполнена");
        }
        used.count += 1;
        used.bytes += bytes;
        Ok(Permit {
            budget: self.clone(),
            bytes,
        })
    }

    /// Stdout can block without losing messages. Shutdown sets `closed` under
    /// this same lock before waking waiters, avoiding a lost notification.
    pub(super) fn reserve(self: &Arc<Self>, bytes: usize, closed: &AtomicBool) -> Result<Permit> {
        if bytes > self.bytes || self.count == 0 {
            bail!("сообщение превышает бюджет транспорта расширений");
        }
        let mut used = self.used.lock().unwrap();
        while !closed.load(Ordering::Acquire) && !self.fits(&used, bytes) {
            #[cfg(test)]
            {
                used.waiters += 1;
            }
            used = self.available.wait(used).unwrap();
            #[cfg(test)]
            {
                used.waiters -= 1;
            }
        }
        if closed.load(Ordering::Acquire) {
            bail!("транспорт расширения закрыт");
        }
        used.count += 1;
        used.bytes += bytes;
        Ok(Permit {
            budget: self.clone(),
            bytes,
        })
    }

    pub(super) fn close(&self, closed: &AtomicBool) {
        let _used = self.used.lock().unwrap();
        closed.store(true, Ordering::Release);
        self.available.notify_all();
    }

    #[cfg(test)]
    pub(super) fn usage(&self) -> (usize, usize) {
        let used = self.used.lock().unwrap();
        (used.count, used.bytes)
    }

    #[cfg(test)]
    pub(super) fn waiters(&self) -> usize {
        self.used.lock().unwrap().waiters
    }
}

pub(super) struct Permit {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Permit {
    pub(super) fn try_grow(&mut self, bytes: usize, closed: &AtomicBool) -> Result<()> {
        let mut used = self.budget.used.lock().unwrap();
        if closed.load(Ordering::Acquire) {
            bail!("транспорт расширения закрыт");
        }
        let extra = bytes.saturating_sub(self.bytes);
        if extra > self.budget.bytes.saturating_sub(used.bytes) {
            bail!("очередь сообщений расширений заполнена");
        }
        used.bytes += extra;
        self.bytes += extra;
        Ok(())
    }

    pub(super) fn shrink(&mut self, bytes: usize) {
        assert!(bytes <= self.bytes);
        let mut used = self.budget.used.lock().unwrap();
        used.bytes -= self.bytes - bytes;
        self.bytes = bytes;
        self.budget.available.notify_all();
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        let mut used = self.budget.used.lock().unwrap();
        used.count -= 1;
        used.bytes -= self.bytes;
        self.budget.available.notify_all();
    }
}
