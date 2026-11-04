//! Validate bounded raw bytes before advancing the remote output cursor.
use anyhow::{Result, bail};
use base64::Engine as _;
use proteus_contracts::contracts::ToolTerminalSnapshot;

pub(super) struct Pending {
    pub snapshot: ToolTerminalSnapshot,
    pub bytes: Option<Vec<u8>>,
    pub reset: bool,
    pub pipe_end: Option<bool>,
}
impl Pending {
    pub fn new(mut snapshot: ToolTerminalSnapshot, cursor: Option<u64>) -> Result<Self> {
        if snapshot.data_base64.len() > 87_384 {
            bail!("Слишком большой ответ терминала");
        }
        let bytes = base64::prelude::BASE64_STANDARD.decode(&snapshot.data_base64)?;
        snapshot.data_base64 = String::new();
        if bytes.len() > 65_536
            || snapshot.start_cursor.checked_add(bytes.len() as u64) != Some(snapshot.next_cursor)
            || snapshot.next_cursor > snapshot.end_cursor
            || ((cursor.is_some_and(|cursor| cursor != snapshot.start_cursor)
                || (cursor.is_none() && snapshot.start_cursor != 0))
                && !snapshot.gap)
        {
            bail!("Нарушен порядок вывода терминала");
        }
        let reset = snapshot.gap;
        Ok(Self {
            snapshot,
            bytes: Some(bytes),
            reset,
            pipe_end: None,
        })
    }

    /// Pipes have no PTY ONLCR. Keep their byte cursor raw while displaying
    /// ordinary log newlines from column zero; preserve CRLF across reads.
    pub fn pipe_newlines(&mut self, previous_cr: bool) {
        if self.pipe_end.is_some() {
            return;
        }
        let bytes = self.bytes.take().expect("pending raw output");
        let mut normalized = Vec::with_capacity(bytes.len());
        let mut cr = previous_cr;
        for byte in bytes {
            if byte == b'\n' && !cr {
                normalized.push(b'\r');
            }
            normalized.push(byte);
            cr = byte == b'\r';
        }
        self.bytes = Some(normalized);
        self.pipe_end = Some(cr);
    }
}
